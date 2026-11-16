use super::{AutoTagCommand, AutoTagEvent};
use crate::domain::auto_tag_rule::AutoTagRule;
use crate::domain::file_tag::FileTag;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// UI-side handle for the auto-tag worker: sends commands, exposes the event
/// stream for the per-frame drain, and stops the thread on drop.
pub struct AutoTagManager {
    command_sender: Sender<AutoTagCommand>,
    pub(crate) event_receiver: Receiver<AutoTagEvent>,
    shutdown: Arc<AtomicBool>,
    worker_handle: Option<std::thread::JoinHandle<Option<()>>>,
    pending_applied: std::vec::IntoIter<(std::path::PathBuf, i64)>,
    ui_ctx: eframe::egui::Context,
}

impl AutoTagManager {
    pub fn start(
        app_state_db: Arc<crate::infrastructure::app_state_db::AppStateDb>,
        rules: Vec<AutoTagRule>,
        defs: Vec<FileTag>,
        ui_ctx: eframe::egui::Context,
    ) -> Self {
        let (command_sender, command_receiver) = channel();
        let (event_sender, event_receiver) = channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let repaint = ui_ctx.clone();

        #[cfg(feature = "notify-watcher")]
        let worker_handle = {
            let worker_shutdown = Arc::clone(&shutdown);
            match crate::spawn_named("auto-tag-watcher", move || {
                super::worker::run_auto_tag(
                    command_receiver,
                    super::AutoTagContext {
                        app_state_db,
                        event_sender,
                        shutdown: worker_shutdown,
                    },
                    rules,
                    defs,
                    ui_ctx,
                );
            }) {
                Ok(handle) => Some(handle),
                Err(error) => {
                    log::error!("[AUTO-TAG] Failed to start watcher thread: {error}");
                    None
                }
            }
        };

        #[cfg(not(feature = "notify-watcher"))]
        let worker_handle = {
            let worker_shutdown = shutdown.clone();
            crate::spawn_named("auto-tag-manual", move || {
                super::reapply::run_manual(
                    command_receiver,
                    super::AutoTagContext {
                        app_state_db,
                        event_sender,
                        shutdown: worker_shutdown,
                    },
                    rules,
                    defs,
                    ui_ctx,
                );
            })
            .ok()
        };

        Self {
            command_sender,
            event_receiver,
            shutdown,
            worker_handle,
            pending_applied: Vec::new().into_iter(),
            ui_ctx: repaint,
        }
    }

    /// Pushes the current rules and tag definitions to the worker (any rule
    /// or tag hierarchy change).
    pub fn send_rules_updated(&self, rules: Vec<AutoTagRule>, defs: Vec<FileTag>) {
        let _ = self
            .command_sender
            .send(AutoTagCommand::RulesUpdated { rules, defs });
    }

    /// Re-scans a rule's folder and tags everything currently inside it.
    pub fn apply_existing(&self, rule_id: i64) {
        let _ = self
            .command_sender
            .send(AutoTagCommand::ApplyExisting { rule_id });
    }

    pub fn reapply_paths(&self, paths: Vec<std::path::PathBuf>) {
        let _ = self
            .command_sender
            .send(AutoTagCommand::ReapplyPaths(paths));
    }

    /// Bound both event count and assignment count, even for one enormous event.
    pub fn drain_events(&mut self) -> Vec<AutoTagEvent> {
        let mut events = Vec::new();
        let mut assignments = Vec::new();
        let mut consumed_events = 0;
        for _ in 0..64 {
            assignments.extend(self.pending_applied.by_ref().take(128 - assignments.len()));
            if assignments.len() == 128 {
                break;
            }
            match self.event_receiver.try_recv() {
                Ok(AutoTagEvent::Applied { assignments: batch }) => {
                    consumed_events += 1;
                    self.pending_applied = batch.into_iter();
                }
                Ok(event) => {
                    consumed_events += 1;
                    events.push(event);
                }
                Err(_) => break,
            }
        }
        let hit_limit = assignments.len() == 128 || consumed_events == 64;
        if !assignments.is_empty() {
            events.push(AutoTagEvent::Applied { assignments });
        }
        if self.pending_applied.len() > 0 || hit_limit {
            self.ui_ctx.request_repaint();
        }
        events
    }
}

impl Drop for AutoTagManager {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = self.command_sender.send(AutoTagCommand::Shutdown);
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn huge_applied_events_are_split_without_dropping_assignments_or_status() {
        let (command_sender, _commands) = channel();
        let (sender, event_receiver) = channel();
        let mut manager = AutoTagManager {
            command_sender,
            event_receiver,
            shutdown: Arc::new(AtomicBool::new(false)),
            worker_handle: None,
            pending_applied: Vec::new().into_iter(),
            ui_ctx: eframe::egui::Context::default(),
        };
        sender
            .send(AutoTagEvent::Applied {
                assignments: (0..1024)
                    .map(|n| (PathBuf::from(format!(r"C:\Data\file-{n}.txt")), 1))
                    .collect(),
            })
            .unwrap();
        sender
            .send(AutoTagEvent::Status {
                rule_id: 1,
                status: super::super::AutoTagRuleStatus::Watching,
            })
            .unwrap();
        sender
            .send(AutoTagEvent::Applied {
                assignments: vec![(PathBuf::from(r"D:\Other\file.txt"), 2)],
            })
            .unwrap();
        let mut paths = std::collections::HashSet::new();
        let mut statuses = 0;
        for _ in 0..16 {
            let events = manager.drain_events();
            if events.is_empty() {
                break;
            }
            let mut assigned = 0;
            for event in events {
                match event {
                    AutoTagEvent::Applied { assignments } => {
                        assigned += assignments.len();
                        for (path, _) in assignments {
                            assert!(paths.insert(path));
                        }
                    }
                    AutoTagEvent::Status { .. } => statuses += 1,
                    _ => panic!("Unexpected event"),
                }
            }
            assert!(assigned <= 128);
        }
        assert_eq!(paths.len(), 1025);
        assert_eq!(statuses, 1);
        assert!(manager.drain_events().is_empty());
    }
}
