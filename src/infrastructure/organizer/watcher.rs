use super::*;
use crate::domain::organizer_rule::validate_rule_set;
use crate::workers::file_operation_worker::OrganizerInFlightRegistry;
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
const WATCH_EVENT_QUEUE_CAPACITY: usize = 1024;

mod queue;
mod supervisor;

use queue::{
    activation_flags_for, process_stable_files, process_watcher_event, queue_rule_paths,
    update_activation_flags,
};
use supervisor::{reconcile_watcher, reset_watcher, WatcherRuntime};

#[cfg(test)]
use queue::{path_is_equal, queue_matching_path};
#[cfg(test)]
use supervisor::{status_for_rule, watched_folders};

pub(super) fn run_organizer(
    command_receiver: Receiver<OrganizerCommand>,
    event_sender: Sender<OrganizerEvent>,
    file_operation_sender: crossbeam_channel::Sender<FileOperationRequest>,
    mut rules: Vec<OrganizerRule>,
    ui_ctx: eframe::egui::Context,
) {
    if validate_rule_set(&rules).is_err() {
        let _ = event_sender.send(OrganizerEvent::Error {
            message: rust_i18n::t!("organizer.error_rule_cycle").to_string(),
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

    for rule in rules.iter().filter(|rule| rule.enabled) {
        queue_rule_paths(rule, &rules, &activation_flags, &paused_rules, &mut pending);
    }

    'organizer: loop {
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
        while let Some(command) = next_command {
            match command {
                OrganizerCommand::SetRules(new_rules) => {
                    if validate_rule_set(&new_rules).is_err() {
                        let _ = event_sender.send(OrganizerEvent::Error {
                            message: rust_i18n::t!("organizer.error_rule_cycle").to_string(),
                        });
                        continue 'organizer;
                    }
                    let previous_rules = std::mem::replace(&mut rules, new_rules);
                    let rules_to_scan =
                        update_activation_flags(&previous_rules, &rules, &mut activation_flags);
                    pending.retain(|_, pending| pending.activation.load(Ordering::Acquire));
                    paused_rules.retain(|rule_id| {
                        rules.iter().any(|rule| rule.id == *rule_id && rule.enabled)
                    });
                    reset_watcher(&mut watcher_runtime);
                    reported_statuses
                        .retain(|rule_id, _| rules.iter().any(|rule| rule.id == *rule_id));
                    for rule_id in rules_to_scan {
                        if let Some(rule) = rules.iter().find(|rule| rule.id == rule_id) {
                            queue_rule_paths(
                                rule,
                                &rules,
                                &activation_flags,
                                &paused_rules,
                                &mut pending,
                            );
                        }
                    }
                }
                OrganizerCommand::RunRuleNow(rule_id) => {
                    let Some(rule) = rules.iter().find(|rule| rule.id == rule_id) else {
                        send_rule_error(&event_sender);
                        continue 'organizer;
                    };
                    if !rule.enabled || paused_rules.contains(&rule_id) {
                        send_rule_error(&event_sender);
                        continue 'organizer;
                    }
                    queue_rule_paths(rule, &rules, &activation_flags, &paused_rules, &mut pending);
                }
                OrganizerCommand::PauseRule(rule_id) => {
                    if let Some(activation) = activation_flags.get(&rule_id) {
                        activation.store(false, Ordering::Release);
                    }
                    activation_flags.insert(rule_id, Arc::new(AtomicBool::new(false)));
                    paused_rules.insert(rule_id);
                    pending.retain(|_, pending| pending.rule.id != rule_id);
                }
                OrganizerCommand::ResumeRule(rule_id) => {
                    paused_rules.remove(&rule_id);
                    if let Some(rule) = rules.iter().find(|rule| rule.id == rule_id) {
                        if rule.enabled {
                            activation_flags.insert(rule_id, Arc::new(AtomicBool::new(true)));
                            queue_rule_paths(
                                rule,
                                &rules,
                                &activation_flags,
                                &paused_rules,
                                &mut pending,
                            );
                        }
                    }
                }
                OrganizerCommand::Refresh => {
                    reset_watcher(&mut watcher_runtime);
                    for rule in rules.iter().filter(|rule| rule.enabled) {
                        queue_rule_paths(
                            rule,
                            &rules,
                            &activation_flags,
                            &paused_rules,
                            &mut pending,
                        );
                    }
                }
                OrganizerCommand::CreateFolder { rule_id, source } => {
                    let Some(rule) = rules.iter().find(|rule| rule.id == rule_id) else {
                        send_rule_error(&event_sender);
                        continue 'organizer;
                    };
                    let folder = if source {
                        &rule.source_folder
                    } else {
                        &rule.destination_folder
                    };
                    if !is_safe_folder_creation_path(folder) || contains_reparse_point(folder) {
                        let _ = event_sender.send(OrganizerEvent::Error {
                            message: rust_i18n::t!("organizer.error_security_path").to_string(),
                        });
                        continue 'organizer;
                    }
                    if let Err(error) = std::fs::create_dir_all(folder) {
                        let _ = event_sender.send(OrganizerEvent::Error {
                            message: rust_i18n::t!("organizer.error_create_folder", reason = error)
                                .to_string(),
                        });
                        continue 'organizer;
                    }
                    if !folder.is_dir() || contains_reparse_point(folder) {
                        let _ = event_sender.send(OrganizerEvent::Error {
                            message: rust_i18n::t!("organizer.error_security_path").to_string(),
                        });
                        continue 'organizer;
                    }
                    reset_watcher(&mut watcher_runtime);
                    if rule.enabled && !paused_rules.contains(&rule_id) {
                        queue_rule_paths(
                            rule,
                            &rules,
                            &activation_flags,
                            &paused_rules,
                            &mut pending,
                        );
                    }
                }
                OrganizerCommand::Shutdown => break 'organizer,
            }
            next_command = match command_receiver.try_recv() {
                Ok(command) => Some(command),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => break 'organizer,
            };
        }

        process_stable_files(
            &mut pending,
            &paused_rules,
            &file_operation_sender,
            &event_sender,
            &in_flight,
        );
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

fn send_rule_error(event_sender: &Sender<OrganizerEvent>) {
    let _ = event_sender.send(OrganizerEvent::Error {
        message: rust_i18n::t!("organizer.error_rule_unavailable").to_string(),
    });
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
