use crate::domain::organizer_operation::OrganizerOperationId;
use crate::domain::organizer_rule::OrganizerRule;
use crate::workers::file_operation_worker::FileOperationRequest;
use std::sync::mpsc::{self, Receiver, Sender};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrganizerRuleStatus {
    Starting,
    Active,
    Disabled,
    Paused,
    SourceUnavailable,
    DestinationUnavailable,
    BothUnavailable,
    Recovering,
}

pub enum OrganizerEvent {
    Status {
        rule_id: i64,
        status: OrganizerRuleStatus,
    },
    OperationSkipped {
        operation_id: OrganizerOperationId,
        rule_id: i64,
        path: std::path::PathBuf,
    },
    OperationFailed {
        operation_id: OrganizerOperationId,
        rule_id: i64,
        path: std::path::PathBuf,
        message: String,
    },
    Error {
        message: String,
    },
}

enum OrganizerCommand {
    SetRules(Vec<OrganizerRule>),
    RunRuleNow(i64),
    PauseRule(i64),
    ResumeRule(i64),
    Refresh,
    CreateFolder { rule_id: i64, source: bool },
    Shutdown,
}

pub struct OrganizerManager {
    command_sender: Sender<OrganizerCommand>,
    pub events: Receiver<OrganizerEvent>,
}

impl OrganizerManager {
    pub(crate) fn start(
        file_operation_sender: crossbeam_channel::Sender<FileOperationRequest>,
        initial_rules: Vec<OrganizerRule>,
        ui_ctx: eframe::egui::Context,
    ) -> Self {
        let (command_sender, command_receiver) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();

        #[cfg(feature = "notify-watcher")]
        {
            let _ = crate::spawn_named("organizer-watcher", move || {
                watcher::run_organizer(
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

    pub fn run_rule_now(&self, rule_id: i64) {
        let _ = self
            .command_sender
            .send(OrganizerCommand::RunRuleNow(rule_id));
    }

    pub fn pause_rule(&self, rule_id: i64) {
        let _ = self
            .command_sender
            .send(OrganizerCommand::PauseRule(rule_id));
    }

    pub fn resume_rule(&self, rule_id: i64) {
        let _ = self
            .command_sender
            .send(OrganizerCommand::ResumeRule(rule_id));
    }

    pub fn refresh(&self) {
        let _ = self.command_sender.send(OrganizerCommand::Refresh);
    }

    pub fn create_missing_folder(&self, rule_id: i64, source: bool) {
        let _ = self
            .command_sender
            .send(OrganizerCommand::CreateFolder { rule_id, source });
    }
}

impl Drop for OrganizerManager {
    fn drop(&mut self) {
        let _ = self.command_sender.send(OrganizerCommand::Shutdown);
    }
}

#[cfg(feature = "notify-watcher")]
mod watcher;
