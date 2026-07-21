use crate::domain::organizer_rule::{preview_rule, validate_rule_set, OrganizerRule};
use crate::infrastructure::windows::shell_operations::{
    organizer_file_snapshot, OrganizerFileSnapshot,
};
use crate::workers::file_operation_worker::FileOperationRequest;
use std::sync::mpsc::{self, Receiver, Sender};

pub enum OrganizerEvent {
    SkippedConflict { path: std::path::PathBuf },
    Error { message: String },
}

enum OrganizerCommand {
    SetRules(Vec<OrganizerRule>),
    Shutdown,
}

pub struct OrganizerManager {
    command_sender: Sender<OrganizerCommand>,
    pub events: Receiver<OrganizerEvent>,
}

impl OrganizerManager {
    pub(crate) fn start(
        file_operation_sender: Sender<FileOperationRequest>,
        initial_rules: Vec<OrganizerRule>,
        ui_ctx: eframe::egui::Context,
    ) -> Self {
        let (command_sender, command_receiver) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();

        #[cfg(feature = "notify-watcher")]
        {
            let _ = crate::spawn_named("organizer-watcher", move || {
                run_organizer(
                    command_receiver,
                    event_sender,
                    file_operation_sender,
                    initial_rules,
                    ui_ctx,
                );
            });
        }

        #[cfg(not(feature = "notify-watcher"))]
        {
            let _ = (
                command_receiver,
                event_sender,
                file_operation_sender,
                initial_rules,
                ui_ctx,
            );
        }

        Self {
            command_sender,
            events: event_receiver,
        }
    }

    pub fn set_rules(&self, rules: Vec<OrganizerRule>) {
        let _ = self.command_sender.send(OrganizerCommand::SetRules(rules));
    }
}

impl Drop for OrganizerManager {
    fn drop(&mut self) {
        let _ = self.command_sender.send(OrganizerCommand::Shutdown);
    }
}

#[cfg(feature = "notify-watcher")]
mod watcher {
    use super::*;
    use notify::{RecursiveMode, Watcher};
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use std::time::{Duration, Instant};

    const STABILITY_DELAY: Duration = Duration::from_secs(2);

    #[derive(Clone)]
    struct PendingFile {
        rule: OrganizerRule,
        activation: Arc<AtomicBool>,
        snapshot: OrganizerFileSnapshot,
        stable_since: Instant,
    }

    pub(super) fn run_organizer(
        command_receiver: Receiver<OrganizerCommand>,
        event_sender: Sender<OrganizerEvent>,
        file_operation_sender: Sender<FileOperationRequest>,
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
        let (watch_event_sender, watch_event_receiver) = mpsc::channel();
        let mut watcher = configure_watcher(&rules, watch_event_sender.clone(), ui_ctx.clone());
        let mut pending = HashMap::new();
        let mut activation_flags = activation_flags_for(&rules);

        for rule in rules.iter().filter(|rule| rule.enabled) {
            queue_rule_paths(rule, &rules, &activation_flags, &mut pending);
        }

        loop {
            while let Ok(event) = watch_event_receiver.try_recv() {
                match event {
                    Ok(event) => {
                        for path in event.paths {
                            queue_matching_path(&rules, &activation_flags, path, &mut pending);
                        }
                    }
                    Err(error) => {
                        let _ = event_sender.send(OrganizerEvent::Error {
                            message: error.to_string(),
                        });
                    }
                }
            }

            process_stable_files(&mut pending, &file_operation_sender, &event_sender);

            match command_receiver.recv_timeout(Duration::from_millis(250)) {
                Ok(OrganizerCommand::SetRules(new_rules)) => {
                    if validate_rule_set(&new_rules).is_err() {
                        let _ = event_sender.send(OrganizerEvent::Error {
                            message: rust_i18n::t!("organizer.error_rule_cycle").to_string(),
                        });
                        continue;
                    }
                    let previous_rules = std::mem::replace(&mut rules, new_rules);
                    let rules_to_scan =
                        update_activation_flags(&previous_rules, &rules, &mut activation_flags);
                    pending.retain(|_, pending| pending.activation.load(Ordering::Acquire));
                    watcher = configure_watcher(&rules, watch_event_sender.clone(), ui_ctx.clone());
                    for rule_id in rules_to_scan {
                        if let Some(rule) = rules.iter().find(|rule| rule.id == rule_id) {
                            queue_rule_paths(rule, &rules, &activation_flags, &mut pending);
                        }
                    }
                }
                Ok(OrganizerCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }

            // Keep the watcher alive for the lifetime of this manager loop.
            let _ = &watcher;
        }
    }

    fn activation_flags_for(rules: &[OrganizerRule]) -> HashMap<i64, Arc<AtomicBool>> {
        rules
            .iter()
            .map(|rule| (rule.id, Arc::new(AtomicBool::new(rule.enabled))))
            .collect()
    }

    fn update_activation_flags(
        previous_rules: &[OrganizerRule],
        rules: &[OrganizerRule],
        activation_flags: &mut HashMap<i64, Arc<AtomicBool>>,
    ) -> Vec<i64> {
        let previous_by_id: HashMap<_, _> =
            previous_rules.iter().map(|rule| (rule.id, rule)).collect();
        let active_rule_ids: HashSet<_> = rules.iter().map(|rule| rule.id).collect();
        let mut rules_to_scan = Vec::new();

        for rule in rules {
            let previous = previous_by_id.get(&rule.id).copied();
            let was_enabled = previous.is_some_and(|previous| previous.enabled);
            let configuration_changed = previous.is_none_or(|previous| {
                previous.source_folder != rule.source_folder
                    || previous.destination_folder != rule.destination_folder
                    || previous.extensions != rule.extensions
            });

            if configuration_changed
                || previous.is_some_and(|previous| previous.enabled != rule.enabled)
            {
                // Pending and already-dispatched moves retain the old token.
                // Disable it before replacing the rule configuration so they
                // cannot execute with stale source, extension, or destination.
                if let Some(previous_activation) = activation_flags.get(&rule.id) {
                    previous_activation.store(false, Ordering::Release);
                }
                activation_flags.insert(rule.id, Arc::new(AtomicBool::new(rule.enabled)));
            } else {
                activation_flags
                    .entry(rule.id)
                    .or_insert_with(|| Arc::new(AtomicBool::new(rule.enabled)));
            }

            if rule.enabled && (!was_enabled || configuration_changed) {
                rules_to_scan.push(rule.id);
            }
        }

        activation_flags.retain(|rule_id, activation| {
            let retained = active_rule_ids.contains(rule_id);
            if !retained {
                activation.store(false, Ordering::Release);
            }
            retained
        });

        rules_to_scan
    }

    fn queue_rule_paths(
        rule: &OrganizerRule,
        rules: &[OrganizerRule],
        activation_flags: &HashMap<i64, Arc<AtomicBool>>,
        pending: &mut HashMap<PathBuf, PendingFile>,
    ) {
        for path in preview_rule(rule) {
            queue_matching_path(rules, activation_flags, path, pending);
        }
    }

    fn configure_watcher(
        rules: &[OrganizerRule],
        event_sender: Sender<notify::Result<notify::Event>>,
        ui_ctx: eframe::egui::Context,
    ) -> Option<notify::RecommendedWatcher> {
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = event_sender.send(event);
            ui_ctx.request_repaint();
        })
        .map_err(|error| log::error!("[ORGANIZER] Failed to create watcher: {error}"))
        .ok()?;

        let mut watched = std::collections::HashSet::new();
        for rule in rules.iter().filter(|rule| rule.enabled) {
            let key = rule.source_folder.to_string_lossy().to_ascii_lowercase();
            if watched.insert(key) {
                if let Err(error) = watcher.watch(&rule.source_folder, RecursiveMode::NonRecursive)
                {
                    log::warn!(
                        "[ORGANIZER] Failed to watch {}: {error}",
                        rule.source_folder.display()
                    );
                }
            }
        }
        Some(watcher)
    }

    fn queue_matching_path(
        rules: &[OrganizerRule],
        activation_flags: &HashMap<i64, Arc<AtomicBool>>,
        path: PathBuf,
        pending: &mut HashMap<PathBuf, PendingFile>,
    ) {
        let Some(rule) = rules
            .iter()
            .find(|rule| rule.enabled && rule.matches(&path))
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
            Some(existing) if existing.snapshot == snapshot => {}
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

    fn process_stable_files(
        pending: &mut HashMap<PathBuf, PendingFile>,
        file_operation_sender: &Sender<FileOperationRequest>,
        event_sender: &Sender<OrganizerEvent>,
    ) {
        let ready: Vec<_> = pending
            .iter()
            .filter(|(_, pending)| pending.stable_since.elapsed() >= STABILITY_DELAY)
            .map(|(path, pending)| (path.clone(), pending.clone()))
            .collect();

        for (path, pending_file) in ready {
            pending.remove(&path);
            let Ok(snapshot) = organizer_file_snapshot(&path) else {
                continue;
            };
            if snapshot != pending_file.snapshot {
                queue_matching_path(
                    std::slice::from_ref(&pending_file.rule),
                    &HashMap::from([(pending_file.rule.id, pending_file.activation.clone())]),
                    path,
                    pending,
                );
                continue;
            }

            if !pending_file.activation.load(Ordering::Acquire) {
                continue;
            }

            let Some(file_name) = path.file_name() else {
                continue;
            };
            if pending_file
                .rule
                .destination_folder
                .join(file_name)
                .exists()
            {
                let _ = event_sender.send(OrganizerEvent::SkippedConflict { path });
                continue;
            }

            if file_operation_sender
                .send(FileOperationRequest::OrganizerMove {
                    path,
                    dest_folder: pending_file.rule.destination_folder,
                    rule_id: pending_file.rule.id,
                    activation: pending_file.activation,
                    expected_snapshot: pending_file.snapshot,
                })
                .is_err()
            {
                let _ = event_sender.send(OrganizerEvent::Error {
                    message: rust_i18n::t!("organizer.error_file_worker_unavailable").to_string(),
                });
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::path::PathBuf;

        fn rule(id: i64, enabled: bool) -> OrganizerRule {
            OrganizerRule {
                id,
                source_folder: PathBuf::from(r"C:\Source"),
                destination_folder: PathBuf::from(r"C:\Destination"),
                extensions: vec!["txt".to_string()],
                enabled,
            }
        }

        #[test]
        fn enabling_a_rule_marks_it_for_a_full_scan() {
            let previous = vec![rule(1, false)];
            let current = vec![rule(1, true)];
            let mut activations = activation_flags_for(&previous);

            let scan_rules = update_activation_flags(&previous, &current, &mut activations);

            assert_eq!(scan_rules, vec![1]);
            assert!(activations[&1].load(Ordering::Acquire));
        }

        #[test]
        fn disabling_a_rule_deactivates_its_pending_work() {
            let previous = vec![rule(1, true)];
            let current = vec![rule(1, false)];
            let mut activations = activation_flags_for(&previous);

            let scan_rules = update_activation_flags(&previous, &current, &mut activations);

            assert!(scan_rules.is_empty());
            assert!(!activations[&1].load(Ordering::Acquire));
        }

        #[test]
        fn changing_a_rule_invalidates_its_previous_activation() {
            let previous = vec![rule(1, true)];
            let mut current = previous.clone();
            current[0].destination_folder = PathBuf::from(r"C:\NewDestination");
            let mut activations = activation_flags_for(&previous);
            let previous_activation = activations[&1].clone();

            let scan_rules = update_activation_flags(&previous, &current, &mut activations);

            assert_eq!(scan_rules, vec![1]);
            assert!(!previous_activation.load(Ordering::Acquire));
            assert!(activations[&1].load(Ordering::Acquire));
        }
    }
}

#[cfg(feature = "notify-watcher")]
use watcher::run_organizer;
