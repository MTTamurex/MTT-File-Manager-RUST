use crate::domain::organizer_operation::OrganizerOperationStatus;
use crate::domain::organizer_rule::{preview_rule, OrganizerRule};
use crate::infrastructure::organizer::OrganizerEvent;
use crate::infrastructure::windows::shell_operations::{
    organizer_file_snapshot, OrganizerFileSnapshot,
};
use crate::workers::file_operation_worker::{
    FileOperationRequest, OrganizerInFlightRegistry, OrganizerUndoExemptionRegistry,
    OrganizerUndoExemptionStatus,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Instant;

use super::{normalize_watched_path, STABILITY_DELAY};

#[derive(Clone)]
pub(super) struct PendingFile {
    pub(super) rule: OrganizerRule,
    pub(super) activation: Arc<AtomicBool>,
    pub(super) snapshot: OrganizerFileSnapshot,
    pub(super) stable_since: Instant,
}

pub(super) fn process_watcher_event(
    event: &notify::Event,
    rules: &[OrganizerRule],
    activation_flags: &HashMap<i64, Arc<AtomicBool>>,
    paused_rules: &HashSet<i64>,
    pending: &mut HashMap<PathBuf, PendingFile>,
) {
    for path in &event.paths {
        queue_matching_path(rules, activation_flags, paused_rules, path.clone(), pending);
    }

    for vacated_destination in event.paths.iter().filter(|path| !path.exists()) {
        let (Some(destination_folder), Some(file_name)) = (
            vacated_destination.parent(),
            vacated_destination.file_name(),
        ) else {
            continue;
        };

        for rule in rules.iter().filter(|rule| {
            rule.enabled
                && !paused_rules.contains(&rule.id)
                && path_is_equal(destination_folder, &rule.destination_folder)
        }) {
            queue_matching_path(
                rules,
                activation_flags,
                paused_rules,
                rule.source_folder.join(file_name),
                pending,
            );
        }
    }
}

pub(super) fn path_is_equal(left: &Path, right: &Path) -> bool {
    normalize_watched_path(left) == normalize_watched_path(right)
}

pub(super) fn queue_rule_paths(
    rule: &OrganizerRule,
    rules: &[OrganizerRule],
    activation_flags: &HashMap<i64, Arc<AtomicBool>>,
    paused_rules: &HashSet<i64>,
    pending: &mut HashMap<PathBuf, PendingFile>,
) {
    for path in preview_rule(rule) {
        queue_matching_path(rules, activation_flags, paused_rules, path, pending);
    }
}

pub(super) fn queue_matching_path(
    rules: &[OrganizerRule],
    activation_flags: &HashMap<i64, Arc<AtomicBool>>,
    paused_rules: &HashSet<i64>,
    path: PathBuf,
    pending: &mut HashMap<PathBuf, PendingFile>,
) {
    let Some(rule) = rules
        .iter()
        .find(|rule| rule.enabled && !paused_rules.contains(&rule.id) && rule.matches(&path))
    else {
        return;
    };
    let Some(activation) = activation_flags.get(&rule.id).cloned() else {
        return;
    };
    let Ok(metadata) = std::fs::metadata(&path) else {
        return;
    };
    if !metadata.is_file() {
        return;
    }
    let Ok(snapshot) = organizer_file_snapshot(&path) else {
        return;
    };

    match pending.get_mut(&path) {
        Some(existing) if existing.snapshot == snapshot => {
            if existing.rule != *rule || !Arc::ptr_eq(&existing.activation, &activation) {
                existing.rule = rule.clone();
                existing.activation = activation;
            }
        }
        Some(existing) => {
            existing.rule = rule.clone();
            existing.activation = activation;
            existing.snapshot = snapshot;
            existing.stable_since = Instant::now();
        }
        None => {
            pending.insert(
                path,
                PendingFile {
                    rule: rule.clone(),
                    activation,
                    snapshot,
                    stable_since: Instant::now(),
                },
            );
        }
    }
}

pub(super) fn process_stable_files(
    pending: &mut HashMap<PathBuf, PendingFile>,
    paused_rules: &HashSet<i64>,
    file_operation_sender: &crossbeam_channel::Sender<FileOperationRequest>,
    event_sender: &Sender<OrganizerEvent>,
    operation_registries: (&OrganizerInFlightRegistry, &OrganizerUndoExemptionRegistry),
    app_state_db: &Arc<crate::infrastructure::app_state_db::AppStateDb>,
    shutdown: &Arc<AtomicBool>,
) -> bool {
    let (in_flight, undo_exemptions) = operation_registries;
    let mut event_sent = false;
    let ready: Vec<_> = pending
        .iter()
        .filter(|(_, pending)| pending.stable_since.elapsed() >= STABILITY_DELAY)
        .map(|(path, pending)| (path.clone(), pending.clone()))
        .collect();

    for (path, pending_file) in ready {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        pending.remove(&path);
        if paused_rules.contains(&pending_file.rule.id) {
            continue;
        }
        let Ok(snapshot) = organizer_file_snapshot(&path) else {
            continue;
        };
        if snapshot != pending_file.snapshot {
            queue_matching_path(
                std::slice::from_ref(&pending_file.rule),
                &HashMap::from([(pending_file.rule.id, pending_file.activation.clone())]),
                paused_rules,
                path,
                pending,
            );
            continue;
        }
        match undo_exemptions.check(&path, snapshot) {
            OrganizerUndoExemptionStatus::Suppressed => continue,
            OrganizerUndoExemptionStatus::Removed => {
                if let Err(error) = app_state_db.remove_organizer_undo_exemption(&path) {
                    log::warn!("[ORGANIZER] Failed to remove stale undo exemption: {error}");
                }
            }
            OrganizerUndoExemptionStatus::Missing => {}
        }

        if !pending_file.activation.load(Ordering::Acquire) {
            continue;
        }
        if !pending_file.rule.destination_folder.is_dir() {
            let mut pending_file = pending_file;
            pending_file.stable_since = Instant::now();
            pending.insert(path, pending_file);
            continue;
        }
        let path_key = normalize_watched_path(&path);
        if in_flight.contains(&path_key) {
            pending.insert(path, pending_file);
            continue;
        }

        let Some(file_name) = path.file_name() else {
            continue;
        };
        let destination = pending_file.rule.destination_folder.join(file_name);
        if destination.exists() {
            let destination_snapshot = organizer_file_snapshot(&destination).ok();
            let registration = match app_state_db.record_terminal_organizer_conflict(
                pending_file.rule.id,
                &path,
                &destination,
                snapshot,
                destination_snapshot,
            ) {
                Ok(registration) => registration,
                Err(error) => {
                    let _ = event_sender.send(OrganizerEvent::Error {
                        message: error.to_string(),
                    });
                    let mut pending_file = pending_file;
                    pending_file.stable_since = Instant::now();
                    pending.insert(path, pending_file);
                    continue;
                }
            };
            let crate::infrastructure::app_state_db::OrganizerConflictRegistration::Created {
                operation_id,
                conflict_id,
            } = registration
            else {
                continue;
            };
            event_sent |= event_sender
                .send(OrganizerEvent::OperationSkipped {
                    operation_id,
                    conflict_id,
                    rule_id: pending_file.rule.id,
                    path,
                    destination,
                })
                .is_ok();
            continue;
        }

        let source_path = path.clone();
        let destination_folder = pending_file.rule.destination_folder.clone();
        let Some(in_flight_guard) = in_flight.try_acquire(path_key) else {
            pending.insert(path, pending_file);
            continue;
        };
        let operation_id = match app_state_db.start_organizer_operation_with_snapshot(
            pending_file.rule.id,
            &path,
            &destination,
            pending_file.snapshot,
        ) {
            Ok(operation_id) => operation_id,
            Err(error) => {
                let _ = event_sender.send(OrganizerEvent::Error {
                    message: error.to_string(),
                });
                let mut pending_file = pending_file;
                pending_file.stable_since = Instant::now();
                pending.insert(path, pending_file);
                continue;
            }
        };
        if file_operation_sender
            .send(FileOperationRequest::OrganizerMove {
                operation_id,
                path,
                dest_folder: destination_folder,
                rule_id: pending_file.rule.id,
                activation: pending_file.activation,
                expected_snapshot: pending_file.snapshot,
                is_undo: false,
                undo_exemptions: undo_exemptions.clone(),
                in_flight: in_flight_guard,
                app_state_db: Arc::clone(app_state_db),
                shutdown: Arc::clone(shutdown),
            })
            .is_err()
        {
            let message = rust_i18n::t!("organizer.error_file_worker_unavailable").to_string();
            if let Err(error) = app_state_db.finish_organizer_operation(
                operation_id,
                OrganizerOperationStatus::Failed,
                Some(&message),
            ) {
                log::error!(
                    "[ORGANIZER] Failed to persist operation {} result: {}",
                    operation_id,
                    error
                );
                let _ = event_sender.send(OrganizerEvent::Error {
                    message: error.to_string(),
                });
            }
            let _ = event_sender.send(OrganizerEvent::OperationFailed {
                operation_id,
                rule_id: pending_file.rule.id,
                path: source_path,
                destination,
                message,
            });
        } else {
            event_sent |= event_sender
                .send(OrganizerEvent::OperationQueued { operation_id })
                .is_ok();
        }
    }
    event_sent
}

pub(super) fn activation_flags_for(rules: &[OrganizerRule]) -> HashMap<i64, Arc<AtomicBool>> {
    rules
        .iter()
        .map(|rule| (rule.id, Arc::new(AtomicBool::new(rule.enabled))))
        .collect()
}

pub(super) fn update_activation_flags(
    previous_rules: &[OrganizerRule],
    rules: &[OrganizerRule],
    activation_flags: &mut HashMap<i64, Arc<AtomicBool>>,
) -> Vec<i64> {
    if previous_rules != rules {
        for activation in activation_flags.values() {
            activation.store(false, Ordering::Release);
        }
        *activation_flags = activation_flags_for(rules);
        rules
            .iter()
            .filter(|rule| rule.enabled)
            .map(|rule| rule.id)
            .collect()
    } else {
        Vec::new()
    }
}
