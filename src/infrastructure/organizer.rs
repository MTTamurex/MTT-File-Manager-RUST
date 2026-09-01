use crate::domain::organizer_rule::OrganizerRule;
use crate::workers::file_operation_worker::FileOperationRequest;
use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

mod protocol;

pub use protocol::{
    OrganizerCommandError, OrganizerCommandResult, OrganizerConflictResolution, OrganizerEvent,
    OrganizerRuleStatus,
};

static NEXT_COMMAND_ID: AtomicU64 = AtomicU64::new(1);

fn allocate_command_id(counter: &AtomicU64) -> Option<OrganizerCommandId> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            (current != 0).then(|| current.wrapping_add(1))
        })
        .ok()
        .map(OrganizerCommandId)
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OrganizerCommandId(u64);

impl OrganizerCommandId {
    fn allocate() -> Option<Self> {
        allocate_command_id(&NEXT_COMMAND_ID)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

struct PendingCommandState {
    running: bool,
    command_ids: HashSet<OrganizerCommandId>,
}

#[derive(Clone)]
struct PendingCommandRegistry {
    state: Arc<Mutex<PendingCommandState>>,
}

impl PendingCommandRegistry {
    fn running() -> Self {
        Self {
            state: Arc::new(Mutex::new(PendingCommandState {
                running: true,
                command_ids: HashSet::new(),
            })),
        }
    }

    fn register_and_send(
        &self,
        command_id: OrganizerCommandId,
        command_sender: &Sender<OrganizerCommand>,
        command: OrganizerCommand,
    ) -> Result<(), OrganizerCommandError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !state.running {
            return Err(OrganizerCommandError::ManagerUnavailable);
        }
        state.command_ids.insert(command_id);
        if command_sender.send(command).is_err() {
            state.command_ids.remove(&command_id);
            state.running = false;
            return Err(OrganizerCommandError::ManagerUnavailable);
        }
        Ok(())
    }

    fn remove(&self, command_id: OrganizerCommandId) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .command_ids
            .remove(&command_id);
    }

    fn stop(&self) -> Vec<OrganizerCommandId> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.running = false;
        state.command_ids.drain().collect()
    }
}

enum OrganizerCommand {
    SetRules {
        command_id: OrganizerCommandId,
        rules: Vec<OrganizerRule>,
    },
    RunRuleNow {
        command_id: OrganizerCommandId,
        rule_id: i64,
    },
    PauseRule {
        command_id: OrganizerCommandId,
        rule_id: i64,
    },
    ResumeRule {
        command_id: OrganizerCommandId,
        rule_id: i64,
    },
    Refresh {
        command_id: OrganizerCommandId,
    },
    CreateFolder {
        command_id: OrganizerCommandId,
        rule_id: i64,
        source: bool,
    },
    ResolveConflict {
        command_id: OrganizerCommandId,
        conflict_id: crate::domain::organizer_conflict::OrganizerConflictId,
        resolution: OrganizerConflictResolution,
    },
    Shutdown,
}

pub struct OrganizerManager {
    command_sender: Sender<OrganizerCommand>,
    watcher_available: bool,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    worker_handle: Option<std::thread::JoinHandle<Option<()>>>,
    pending_commands: PendingCommandRegistry,
    events: Receiver<OrganizerEvent>,
}

impl OrganizerManager {
    pub(crate) fn start(
        file_operation_sender: crossbeam_channel::Sender<FileOperationRequest>,
        app_state_db: Arc<crate::infrastructure::app_state_db::AppStateDb>,
        initial_rules: Vec<OrganizerRule>,
        ui_ctx: eframe::egui::Context,
    ) -> Self {
        let (command_sender, command_receiver) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();
        let pending_commands = PendingCommandRegistry::running();
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));

        #[cfg(feature = "notify-watcher")]
        let worker_handle = {
            let worker_pending_commands = pending_commands.clone();
            let worker_shutdown = shutdown.clone();
            let startup_event_sender = event_sender.clone();
            let startup_ui_ctx = ui_ctx.clone();
            match crate::spawn_named("organizer-watcher", move || {
                watcher::run_organizer(
                    command_receiver,
                    event_sender,
                    (file_operation_sender, app_state_db),
                    initial_rules,
                    ui_ctx,
                    worker_pending_commands,
                    worker_shutdown,
                );
            }) {
                Ok(worker_handle) => Some(worker_handle),
                Err(error) => {
                    pending_commands.stop();
                    log::error!("[ORGANIZER] Failed to start watcher thread: {error}");
                    let _ = startup_event_sender.send(OrganizerEvent::Error {
                        message: OrganizerCommandError::ManagerUnavailable.to_string(),
                    });
                    startup_ui_ctx.request_repaint();
                    None
                }
            }
        };

        #[cfg(not(feature = "notify-watcher"))]
        let worker_handle = {
            let _ = (
                command_receiver,
                event_sender,
                file_operation_sender,
                app_state_db,
                initial_rules,
                ui_ctx,
            );
            None
        };

        let watcher_available = worker_handle.is_some();
        if !watcher_available {
            pending_commands.stop();
        }

        Self {
            command_sender,
            watcher_available,
            shutdown,
            worker_handle,
            pending_commands,
            events: event_receiver,
        }
    }

    fn enqueue(
        &self,
        command: impl FnOnce(OrganizerCommandId) -> OrganizerCommand,
    ) -> Result<OrganizerCommandId, OrganizerCommandError> {
        if !self.watcher_available {
            return Err(OrganizerCommandError::ManagerUnavailable);
        }
        let command_id =
            OrganizerCommandId::allocate().ok_or(OrganizerCommandError::CommandIdExhausted)?;
        self.pending_commands.register_and_send(
            command_id,
            &self.command_sender,
            command(command_id),
        )?;
        Ok(command_id)
    }

    pub fn set_rules(
        &self,
        rules: Vec<OrganizerRule>,
    ) -> Result<OrganizerCommandId, OrganizerCommandError> {
        self.enqueue(move |command_id| OrganizerCommand::SetRules { command_id, rules })
    }

    pub fn run_rule_now(&self, rule_id: i64) -> Result<OrganizerCommandId, OrganizerCommandError> {
        self.enqueue(move |command_id| OrganizerCommand::RunRuleNow {
            command_id,
            rule_id,
        })
    }

    pub fn pause_rule(&self, rule_id: i64) -> Result<OrganizerCommandId, OrganizerCommandError> {
        self.enqueue(move |command_id| OrganizerCommand::PauseRule {
            command_id,
            rule_id,
        })
    }

    pub fn resume_rule(&self, rule_id: i64) -> Result<OrganizerCommandId, OrganizerCommandError> {
        self.enqueue(move |command_id| OrganizerCommand::ResumeRule {
            command_id,
            rule_id,
        })
    }

    pub fn refresh(&self) -> Result<OrganizerCommandId, OrganizerCommandError> {
        self.enqueue(|command_id| OrganizerCommand::Refresh { command_id })
    }

    pub fn create_missing_folder(
        &self,
        rule_id: i64,
        source: bool,
    ) -> Result<OrganizerCommandId, OrganizerCommandError> {
        self.enqueue(move |command_id| OrganizerCommand::CreateFolder {
            command_id,
            rule_id,
            source,
        })
    }

    pub fn resolve_conflict(
        &self,
        conflict_id: crate::domain::organizer_conflict::OrganizerConflictId,
        resolution: OrganizerConflictResolution,
    ) -> Result<OrganizerCommandId, OrganizerCommandError> {
        self.enqueue(move |command_id| OrganizerCommand::ResolveConflict {
            command_id,
            conflict_id,
            resolution,
        })
    }

    pub(crate) fn try_recv_event(&self) -> Result<OrganizerEvent, mpsc::TryRecvError> {
        self.events.try_recv()
    }

    #[cfg(test)]
    fn recv_event_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<OrganizerEvent, mpsc::RecvTimeoutError> {
        self.events.recv_timeout(timeout)
    }
}

impl Drop for OrganizerManager {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        self.pending_commands.stop();
        if self.watcher_available {
            let _ = self.command_sender.send(OrganizerCommand::Shutdown);
        }
        if let Some(worker_handle) = self.worker_handle.take() {
            if worker_handle.is_finished() {
                let _ = worker_handle.join();
            }
        }
    }
}

#[cfg(feature = "notify-watcher")]
mod watcher;

#[cfg(test)]
#[path = "organizer/manager_tests.rs"]
mod tests;
