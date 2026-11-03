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
            let _ = (
                command_receiver,
                event_sender,
                app_state_db,
                rules,
                defs,
                ui_ctx,
            );
            None
        };

        Self {
            command_sender,
            event_receiver,
            shutdown,
            worker_handle,
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

    /// Drains every pending worker event without blocking.
    pub fn drain_events(&self) -> Vec<AutoTagEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_receiver.try_recv() {
            events.push(event);
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
