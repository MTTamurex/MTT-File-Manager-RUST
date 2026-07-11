use crate::domain::organizer_rule::{preview_rule, OrganizerRule};
use crate::workers::file_operation_worker::FileOperationRequest;
use std::sync::mpsc::{self, Receiver, Sender};

pub enum OrganizerEvent {
    SkippedConflict { path: std::path::PathBuf },
    Error { message: String },
}

enum OrganizerCommand {
    SetRules(Vec<OrganizerRule>),
    RunNow(i64),
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

    pub fn run_now(&self, rule_id: i64) {
        let _ = self.command_sender.send(OrganizerCommand::RunNow(rule_id));
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
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::{Duration, Instant, SystemTime};

    const STABILITY_DELAY: Duration = Duration::from_secs(2);

    #[derive(Clone)]
    struct PendingFile {
        rule: OrganizerRule,
        size: u64,
        modified: Option<SystemTime>,
        stable_since: Instant,
    }

    pub(super) fn run_organizer(
        command_receiver: Receiver<OrganizerCommand>,
        event_sender: Sender<OrganizerEvent>,
        file_operation_sender: Sender<FileOperationRequest>,
        mut rules: Vec<OrganizerRule>,
        ui_ctx: eframe::egui::Context,
    ) {
        let (watch_event_sender, watch_event_receiver) = mpsc::channel();
        let mut watcher = configure_watcher(&rules, watch_event_sender.clone(), ui_ctx.clone());
        let mut pending = HashMap::new();

        loop {
            while let Ok(event) = watch_event_receiver.try_recv() {
                match event {
                    Ok(event) => {
                        for path in event.paths {
                            queue_matching_path(&rules, path, &mut pending);
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
                    rules = new_rules;
                    pending.clear();
                    watcher = configure_watcher(&rules, watch_event_sender.clone(), ui_ctx.clone());
                }
                Ok(OrganizerCommand::RunNow(rule_id)) => {
                    if let Some(rule) = rules.iter().find(|rule| rule.id == rule_id && rule.enabled)
                    {
                        for path in preview_rule(rule) {
                            queue_matching_path(&rules, path, &mut pending);
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
        path: PathBuf,
        pending: &mut HashMap<PathBuf, PendingFile>,
    ) {
        let Some(rule) = rules
            .iter()
            .find(|rule| rule.enabled && rule.matches(&path))
        else {
            return;
        };
        let Ok(metadata) = std::fs::metadata(&path) else {
            return;
        };
        if !metadata.is_file() {
            return;
        }

        let modified = metadata.modified().ok();
        match pending.get_mut(&path) {
            Some(existing) if existing.size == metadata.len() && existing.modified == modified => {}
            Some(existing) => {
                existing.rule = rule.clone();
                existing.size = metadata.len();
                existing.modified = modified;
                existing.stable_since = Instant::now();
            }
            None => {
                pending.insert(
                    path,
                    PendingFile {
                        rule: rule.clone(),
                        size: metadata.len(),
                        modified,
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
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            if metadata.len() != pending_file.size
                || metadata.modified().ok() != pending_file.modified
            {
                queue_matching_path(std::slice::from_ref(&pending_file.rule), path, pending);
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
                })
                .is_err()
            {
                let _ = event_sender.send(OrganizerEvent::Error {
                    message: "O worker de operações de arquivo não está disponível".to_string(),
                });
            }
        }
    }
}

#[cfg(feature = "notify-watcher")]
use watcher::run_organizer;
