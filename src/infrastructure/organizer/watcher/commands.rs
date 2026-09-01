use super::queue::{queue_rule_paths, update_activation_flags, PendingFile};
use super::supervisor::{reset_watcher, WatcherRuntime};
use super::*;
use crate::domain::organizer_rule::validate_rule_set;
use std::path::PathBuf;

pub(super) struct CommandContext<'a> {
    pub(super) rules: &'a mut Vec<OrganizerRule>,
    pub(super) activation_flags: &'a mut HashMap<i64, Arc<AtomicBool>>,
    pub(super) paused_rules: &'a mut HashSet<i64>,
    pub(super) pending: &'a mut HashMap<PathBuf, PendingFile>,
    pub(super) watcher_runtime: &'a mut WatcherRuntime,
    pub(super) reported_statuses: &'a mut HashMap<i64, OrganizerRuleStatus>,
    pub(super) event_sender: &'a Sender<OrganizerEvent>,
    pub(super) ui_ctx: &'a eframe::egui::Context,
    pub(super) pending_commands: &'a PendingCommandRegistry,
}

impl CommandContext<'_> {
    pub(super) fn process(&mut self, command: OrganizerCommand) -> bool {
        match command {
            OrganizerCommand::SetRules {
                command_id,
                rules: new_rules,
            } => self.set_rules(command_id, new_rules),
            OrganizerCommand::RunRuleNow {
                command_id,
                rule_id,
            } => self.run_rule_now(command_id, rule_id),
            OrganizerCommand::PauseRule {
                command_id,
                rule_id,
            } => self.pause_rule(command_id, rule_id),
            OrganizerCommand::ResumeRule {
                command_id,
                rule_id,
            } => self.resume_rule(command_id, rule_id),
            OrganizerCommand::Refresh { command_id } => self.refresh(command_id),
            OrganizerCommand::CreateFolder {
                command_id,
                rule_id,
                source,
            } => self.create_folder(command_id, rule_id, source),
            OrganizerCommand::Shutdown => true,
        }
    }

    fn set_rules(&mut self, command_id: OrganizerCommandId, new_rules: Vec<OrganizerRule>) -> bool {
        if let Err(error) = validate_runtime_rules(&new_rules) {
            return self.respond(command_id, Err(error));
        }

        let previous_rules = std::mem::replace(self.rules, new_rules);
        let rules_to_scan =
            update_activation_flags(&previous_rules, self.rules, self.activation_flags);
        self.pending
            .retain(|_, pending| pending.activation.load(Ordering::Acquire));
        self.paused_rules.retain(|rule_id| {
            self.rules
                .iter()
                .any(|rule| rule.id == *rule_id && rule.enabled)
        });
        reset_watcher(self.watcher_runtime);
        self.reported_statuses
            .retain(|rule_id, _| self.rules.iter().any(|rule| rule.id == *rule_id));
        for rule_id in rules_to_scan {
            if let Some(rule) = self.rules.iter().find(|rule| rule.id == rule_id) {
                queue_rule_paths(
                    rule,
                    self.rules,
                    self.activation_flags,
                    self.paused_rules,
                    self.pending,
                );
            }
        }
        self.respond(
            command_id,
            Ok(OrganizerCommandResult::RulesUpdated {
                rule_count: self.rules.len(),
            }),
        )
    }

    fn run_rule_now(&mut self, command_id: OrganizerCommandId, rule_id: i64) -> bool {
        let Some(rule) = self.rules.iter().find(|rule| rule.id == rule_id) else {
            return self.rule_unavailable(command_id);
        };
        if !rule.enabled || self.paused_rules.contains(&rule_id) {
            return self.rule_unavailable(command_id);
        }
        queue_rule_paths(
            rule,
            self.rules,
            self.activation_flags,
            self.paused_rules,
            self.pending,
        );
        self.respond(
            command_id,
            Ok(OrganizerCommandResult::RuleRunQueued { rule_id }),
        )
    }

    fn pause_rule(&mut self, command_id: OrganizerCommandId, rule_id: i64) -> bool {
        if !self
            .rules
            .iter()
            .any(|rule| rule.id == rule_id && rule.enabled)
        {
            return self.rule_unavailable(command_id);
        }
        if let Some(activation) = self.activation_flags.get(&rule_id) {
            activation.store(false, Ordering::Release);
        }
        self.activation_flags
            .insert(rule_id, Arc::new(AtomicBool::new(false)));
        self.paused_rules.insert(rule_id);
        self.pending.retain(|_, pending| pending.rule.id != rule_id);
        self.respond(
            command_id,
            Ok(OrganizerCommandResult::RulePaused { rule_id }),
        )
    }

    fn resume_rule(&mut self, command_id: OrganizerCommandId, rule_id: i64) -> bool {
        let Some(rule) = self
            .rules
            .iter()
            .find(|rule| rule.id == rule_id && rule.enabled)
        else {
            return self.rule_unavailable(command_id);
        };
        self.paused_rules.remove(&rule_id);
        self.activation_flags
            .insert(rule_id, Arc::new(AtomicBool::new(true)));
        queue_rule_paths(
            rule,
            self.rules,
            self.activation_flags,
            self.paused_rules,
            self.pending,
        );
        self.respond(
            command_id,
            Ok(OrganizerCommandResult::RuleResumed { rule_id }),
        )
    }

    fn refresh(&mut self, command_id: OrganizerCommandId) -> bool {
        reset_watcher(self.watcher_runtime);
        for rule in self.rules.iter().filter(|rule| rule.enabled) {
            queue_rule_paths(
                rule,
                self.rules,
                self.activation_flags,
                self.paused_rules,
                self.pending,
            );
        }
        self.respond(
            command_id,
            Ok(OrganizerCommandResult::RefreshQueued {
                enabled_rule_count: self.rules.iter().filter(|rule| rule.enabled).count(),
            }),
        )
    }

    fn create_folder(
        &mut self,
        command_id: OrganizerCommandId,
        rule_id: i64,
        source: bool,
    ) -> bool {
        let Some(rule) = self.rules.iter().find(|rule| rule.id == rule_id) else {
            return self.rule_unavailable(command_id);
        };
        let folder = if source {
            &rule.source_folder
        } else {
            &rule.destination_folder
        };
        if !is_safe_folder_creation_path(folder) || contains_reparse_point(folder) {
            return self.respond(command_id, Err(OrganizerCommandError::SecurityViolation));
        }
        if let Err(error) = std::fs::create_dir_all(folder) {
            return self.respond(
                command_id,
                Err(OrganizerCommandError::FolderCreationFailed {
                    reason: error.to_string(),
                }),
            );
        }
        if !folder.is_dir() || contains_reparse_point(folder) {
            return self.respond(command_id, Err(OrganizerCommandError::SecurityViolation));
        }

        let folder = folder.clone();
        reset_watcher(self.watcher_runtime);
        if rule.enabled && !self.paused_rules.contains(&rule_id) {
            queue_rule_paths(
                rule,
                self.rules,
                self.activation_flags,
                self.paused_rules,
                self.pending,
            );
        }
        self.respond(
            command_id,
            Ok(OrganizerCommandResult::FolderReady {
                rule_id,
                source,
                path: folder,
            }),
        )
    }

    fn rule_unavailable(&self, command_id: OrganizerCommandId) -> bool {
        self.respond(command_id, Err(OrganizerCommandError::RuleUnavailable))
    }

    fn respond(
        &self,
        command_id: OrganizerCommandId,
        result: Result<OrganizerCommandResult, OrganizerCommandError>,
    ) -> bool {
        if self
            .event_sender
            .send(OrganizerEvent::CommandResult { command_id, result })
            .is_ok()
        {
            self.pending_commands.remove(command_id);
            self.ui_ctx.request_repaint();
            false
        } else {
            true
        }
    }
}

pub(super) fn validate_runtime_rules(rules: &[OrganizerRule]) -> Result<(), OrganizerCommandError> {
    let mut rule_ids = HashSet::new();
    for rule in rules {
        if !rule_ids.insert(rule.id) {
            return Err(OrganizerCommandError::DuplicateRuleId { rule_id: rule.id });
        }
    }
    validate_rule_set(rules).map_err(OrganizerCommandError::InvalidRules)
}
