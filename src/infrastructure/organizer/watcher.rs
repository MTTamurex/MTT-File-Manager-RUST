use super::*;
use crate::workers::file_operation_worker::{
    OrganizerInFlightRegistry, OrganizerUndoExemptionRegistry,
};
use std::collections::{HashMap, HashSet};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

const STABILITY_DELAY: Duration = Duration::from_secs(2);
const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const INITIAL_WATCHER_RETRY_DELAY: Duration = Duration::from_secs(1);
const MAX_WATCHER_RETRY_DELAY: Duration = Duration::from_secs(30);
const MAX_WATCH_EVENTS_PER_TICK: usize = 128;
const MAX_COMMANDS_PER_TICK: usize = 64;
const WATCH_EVENT_QUEUE_CAPACITY: usize = 1024;

mod commands;
mod queue;
mod supervisor;

use commands::{validate_runtime_rules, CommandContext};
use queue::{activation_flags_for, process_stable_files, process_watcher_event, queue_rule_paths};
use supervisor::{reconcile_watcher, WatcherRuntime};

#[cfg(test)]
use queue::{path_is_equal, queue_matching_path, update_activation_flags};
#[cfg(test)]
use supervisor::{status_for_rule, watched_folders};

pub(super) fn run_organizer(
    command_receiver: Receiver<OrganizerCommand>,
    event_sender: Sender<OrganizerEvent>,
    operation_services: (
        crossbeam_channel::Sender<FileOperationRequest>,
        Arc<crate::infrastructure::app_state_db::AppStateDb>,
    ),
    mut rules: Vec<OrganizerRule>,
    ui_ctx: eframe::egui::Context,
    pending_commands: PendingCommandRegistry,
    shutdown: Arc<AtomicBool>,
) {
    let (file_operation_sender, app_state_db) = operation_services;
    let _pending_command_guard = PendingCommandFailureGuard {
        pending_commands: pending_commands.clone(),
        event_sender: event_sender.clone(),
        ui_ctx: ui_ctx.clone(),
    };
    if let Err(error) = validate_runtime_rules(&rules) {
        let _ = event_sender.send(OrganizerEvent::Error {
            message: error.to_string(),
        });
        for rule in &mut rules {
            rule.enabled = false;
        }
    }

    let (watch_event_sender, watch_event_receiver) = mpsc::sync_channel(WATCH_EVENT_QUEUE_CAPACITY);
    let watch_overflowed = Arc::new(AtomicBool::new(false));
    let mut watcher_runtime =
        WatcherRuntime::new(watch_event_sender, Arc::clone(&watch_overflowed));
    let mut next_health_check = Instant::now();
    let mut reported_statuses = HashMap::new();
    let mut paused_rules = HashSet::new();
    let mut pending = HashMap::new();
    let mut activation_flags = activation_flags_for(&rules);
    let in_flight = OrganizerInFlightRegistry::default();
    let undo_exemptions = match app_state_db.list_organizer_undo_exemptions() {
        Ok(exemptions) => OrganizerUndoExemptionRegistry::new(exemptions),
        Err(error) => {
            log::warn!("[ORGANIZER] Failed to load undo exemptions: {error}");
            OrganizerUndoExemptionRegistry::default()
        }
    };

    for rule in rules.iter().filter(|rule| rule.enabled) {
        queue_rule_paths(rule, &rules, &activation_flags, &paused_rules, &mut pending);
    }

    'organizer: loop {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        if Instant::now() >= next_health_check {
            let recovered_rules = reconcile_watcher(
                &rules,
                &paused_rules,
                &mut watcher_runtime,
                &ui_ctx,
                &event_sender,
                &mut reported_statuses,
            );
            for rule_id in recovered_rules {
                if let Some(rule) = rules.iter().find(|rule| rule.id == rule_id) {
                    queue_rule_paths(rule, &rules, &activation_flags, &paused_rules, &mut pending);
                }
            }
            next_health_check = Instant::now() + HEALTH_CHECK_INTERVAL;
        }

        for _ in 0..MAX_WATCH_EVENTS_PER_TICK {
            let Ok(event) = watch_event_receiver.try_recv() else {
                break;
            };
            match event {
                Ok(event) => process_watcher_event(
                    &event,
                    &rules,
                    &activation_flags,
                    &paused_rules,
                    &mut pending,
                ),
                Err(error) => {
                    watcher_runtime.ready = false;
                    watcher_runtime.next_retry = Instant::now();
                    let _ = event_sender.send(OrganizerEvent::Error {
                        message: error.to_string(),
                    });
                }
            }
        }
        if watch_overflowed.swap(false, Ordering::AcqRel) {
            for rule in rules.iter().filter(|rule| rule.enabled) {
                queue_rule_paths(rule, &rules, &activation_flags, &paused_rules, &mut pending);
            }
        }

        let mut next_command = match command_receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(command) => Some(command),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => None,
        };
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        let mut processed_commands = 0;
        {
            let mut command_context = CommandContext {
                rules: &mut rules,
                activation_flags: &mut activation_flags,
                paused_rules: &mut paused_rules,
                pending: &mut pending,
                watcher_runtime: &mut watcher_runtime,
                reported_statuses: &mut reported_statuses,
                event_sender: &event_sender,
                ui_ctx: &ui_ctx,
                pending_commands: &pending_commands,
                app_state_db: &app_state_db,
                in_flight: &in_flight,
                undo_exemptions: &undo_exemptions,
                file_operation_sender: &file_operation_sender,
                shutdown: &shutdown,
            };
            while let Some(command) = next_command {
                if shutdown.load(Ordering::Acquire) {
                    break 'organizer;
                }
                processed_commands += 1;
                if command_context.process(command) {
                    break 'organizer;
                }
                if processed_commands >= MAX_COMMANDS_PER_TICK {
                    break;
                }
                next_command = match command_receiver.try_recv() {
                    Ok(command) => Some(command),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(mpsc::TryRecvError::Disconnected) => break 'organizer,
                };
            }
        }

        if process_stable_files(
            &mut pending,
            &paused_rules,
            &file_operation_sender,
            &event_sender,
            (&in_flight, &undo_exemptions),
            &app_state_db,
            &shutdown,
        ) {
            ui_ctx.request_repaint();
        }
    }
}

struct PendingCommandFailureGuard {
    pending_commands: PendingCommandRegistry,
    event_sender: Sender<OrganizerEvent>,
    ui_ctx: eframe::egui::Context,
}

impl Drop for PendingCommandFailureGuard {
    fn drop(&mut self) {
        let mut sent_failure = false;
        for command_id in self.pending_commands.stop() {
            sent_failure |= self
                .event_sender
                .send(OrganizerEvent::CommandResult {
                    command_id,
                    result: Err(OrganizerCommandError::ManagerUnavailable),
                })
                .is_ok();
        }
        if sent_failure {
            self.ui_ctx.request_repaint();
        }
    }
}

fn is_safe_folder_creation_path(path: &std::path::Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    if path.components().any(|component| {
        matches!(
            component,
            std::path::Component::CurDir | std::path::Component::ParentDir
        )
    }) {
        return false;
    }
    let path = path.to_string_lossy();
    !path.starts_with(r"\\?\") && !path.starts_with(r"\\.\")
}

fn contains_reparse_point(path: &std::path::Path) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    let mut current = Some(path);
    while let Some(candidate) = current {
        if std::fs::symlink_metadata(candidate)
            .is_ok_and(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        {
            return true;
        }
        current = candidate.parent();
    }
    false
}

fn normalize_watched_path(path: &std::path::Path) -> String {
    let normalized = path
        .to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase();
    if let Some(stripped) = normalized.strip_prefix(r"\\?\unc\") {
        return format!(r"\\{stripped}");
    }
    normalized
        .strip_prefix(r"\\?\")
        .or_else(|| normalized.strip_prefix(r"\\.\"))
        .unwrap_or(&normalized)
        .to_string()
}

#[cfg(test)]
#[path = "watcher/organizer_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "watcher/queue_tests.rs"]
mod queue_tests;
