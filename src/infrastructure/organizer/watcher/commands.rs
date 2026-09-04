use super::queue::{queue_matching_path, queue_rule_paths, update_activation_flags, PendingFile};
use super::supervisor::{reset_watcher, WatcherRuntime};
use super::*;
use crate::domain::organizer_conflict::{
    validate_organizer_conflict_name, OrganizerConflictStatus,
};
use crate::domain::organizer_operation::{
    OrganizerOperationId, OrganizerOperationStatus, OrganizerOperationType,
};
use crate::domain::organizer_rule::validate_rule_set;
use crate::domain::organizer_rule::OrganizerConflictPolicy;
use crate::infrastructure::app_state_db::{
    AppStateDb, OrganizerConflictRecord, OrganizerOperationDbError,
};
use crate::infrastructure::windows::shell_operations::{
    move_organizer_file_without_replace_guarded_by, organizer_file_snapshot, OrganizerFileSnapshot,
};
use crate::workers::file_operation_worker::{
    FileOperationRequest, OrganizerInFlightRegistry, OrganizerUndoExemptionRegistry,
};
use std::path::PathBuf;
use std::sync::{atomic::AtomicBool, Arc};

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
    pub(super) app_state_db: &'a std::sync::Arc<AppStateDb>,
    pub(super) in_flight: &'a OrganizerInFlightRegistry,
    pub(super) undo_exemptions: &'a OrganizerUndoExemptionRegistry,
    pub(super) file_operation_sender:
        &'a crossbeam_channel::Sender<crate::workers::file_operation_worker::FileOperationRequest>,
    pub(super) shutdown: &'a Arc<AtomicBool>,
}

impl CommandContext<'_> {
    pub(super) fn process(&mut self, command: OrganizerCommand) -> bool {
        match command {
            OrganizerCommand::SetRules {
                command_id,
                rules: new_rules,
            } => self.set_rules(command_id, new_rules),
            OrganizerCommand::PauseRule {
                command_id,
                rule_id,
            } => self.pause_rule(command_id, rule_id),
            OrganizerCommand::ResumeRule {
                command_id,
                rule_id,
            } => self.resume_rule(command_id, rule_id),
            OrganizerCommand::CreateFolder {
                command_id,
                rule_id,
                source,
            } => self.create_folder(command_id, rule_id, source),
            OrganizerCommand::ResolveConflict {
                command_id,
                conflict_id,
                resolution,
            } => self.resolve_conflict(command_id, conflict_id, resolution),
            OrganizerCommand::RetryOperation {
                command_id,
                operation_id,
            } => self.retry_operation(command_id, operation_id),
            OrganizerCommand::UndoOperation {
                command_id,
                operation_id,
            } => self.undo_operation(command_id, operation_id),
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

    fn resolve_conflict(
        &mut self,
        command_id: OrganizerCommandId,
        conflict_id: crate::domain::organizer_conflict::OrganizerConflictId,
        resolution: OrganizerConflictResolution,
    ) -> bool {
        let conflict = match self.app_state_db.get_organizer_conflict(conflict_id) {
            Ok(Some(conflict)) if conflict.status == OrganizerConflictStatus::Pending => conflict,
            Ok(Some(_)) | Ok(None) => {
                return self.respond(command_id, Err(OrganizerCommandError::ConflictUnavailable))
            }
            Err(error) => {
                return self.respond(
                    command_id,
                    Err(OrganizerCommandError::ConflictResolutionFailed {
                        reason: error.to_string(),
                    }),
                )
            }
        };

        let result = match resolution {
            OrganizerConflictResolution::RenameSource { new_name } => self
                .rename_conflict_source(&conflict, &new_name)
                .map(
                    |(old_path, new_path)| OrganizerCommandResult::ConflictResolved {
                        conflict_id,
                        old_path,
                        new_path,
                    },
                ),
            OrganizerConflictResolution::RenameDestination { new_name } => {
                self.rename_conflict_destination(&conflict, &new_name).map(
                    |(old_path, new_path)| OrganizerCommandResult::ConflictResolved {
                        conflict_id,
                        old_path,
                        new_path,
                    },
                )
            }
            OrganizerConflictResolution::Cancel => self
                .cancel_conflict(conflict.conflict_id)
                .map(|()| OrganizerCommandResult::ConflictCancelled { conflict_id }),
        };
        self.respond(command_id, result)
    }

    fn retry_operation(
        &mut self,
        command_id: OrganizerCommandId,
        operation_id: OrganizerOperationId,
    ) -> bool {
        let record = match self.app_state_db.get_organizer_operation(operation_id) {
            Ok(Some(record)) => record,
            Ok(None) => {
                return self.respond(command_id, Err(OrganizerCommandError::RetryUnavailable))
            }
            Err(error) => {
                return self.respond(
                    command_id,
                    Err(OrganizerCommandError::OperationDispatchFailed {
                        reason: error.to_string(),
                    }),
                )
            }
        };
        if record.operation_type == OrganizerOperationType::Undo
            || !matches!(
                record.status,
                OrganizerOperationStatus::Failed
                    | OrganizerOperationStatus::Skipped
                    | OrganizerOperationStatus::Cancelled
            )
        {
            return self.respond(command_id, Err(OrganizerCommandError::RetryUnavailable));
        }
        let Some(rule_id) = record.rule_id else {
            return self.respond(command_id, Err(OrganizerCommandError::RetryUnavailable));
        };
        let Some(expected_snapshot) = record.source_snapshot_before else {
            return self.respond(command_id, Err(OrganizerCommandError::RetryUnavailable));
        };
        let Some(rule) = self
            .rules
            .iter()
            .find(|rule| rule.id == rule_id && rule.enabled)
            .cloned()
        else {
            return self.respond(command_id, Err(OrganizerCommandError::RetryUnavailable));
        };
        if self.paused_rules.contains(&rule_id) {
            return self.respond(command_id, Err(OrganizerCommandError::RetryUnavailable));
        }

        let source = record
            .effective_source_path
            .clone()
            .unwrap_or_else(|| record.source_path.clone());
        let Ok(source) = safe_organizer_path(&source) else {
            return self.respond(command_id, Err(OrganizerCommandError::RetryUnavailable));
        };
        let Ok(current_snapshot) = organizer_file_snapshot(&source) else {
            return self.respond(command_id, Err(OrganizerCommandError::RetryUnavailable));
        };
        if current_snapshot != expected_snapshot || !rule.matches(&source) {
            return self.respond(command_id, Err(OrganizerCommandError::RetryUnavailable));
        }
        let Some(file_name) = source.file_name() else {
            return self.respond(command_id, Err(OrganizerCommandError::RetryUnavailable));
        };
        let Ok(destination_folder) = safe_organizer_path(&rule.destination_folder) else {
            return self.respond(command_id, Err(OrganizerCommandError::RetryUnavailable));
        };
        if !destination_folder.is_dir() {
            return self.respond(command_id, Err(OrganizerCommandError::RetryUnavailable));
        }
        let destination = destination_folder.join(file_name);
        let path_key = super::normalize_watched_path(&source);
        let Some(in_flight_guard) = self.in_flight.try_acquire(path_key) else {
            return self.respond(command_id, Err(OrganizerCommandError::RetryUnavailable));
        };
        let new_operation_id = match self.app_state_db.create_retry_organizer_operation(
            operation_id,
            rule_id,
            &source,
            &destination,
            expected_snapshot,
        ) {
            Ok(new_operation_id) => new_operation_id,
            Err(error) => {
                return self.respond(command_id, Err(retry_command_error(error)));
            }
        };
        let activation = self
            .activation_flags
            .get(&rule_id)
            .cloned()
            .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        if self
            .file_operation_sender
            .send(FileOperationRequest::OrganizerMove {
                operation_id: new_operation_id,
                path: source.clone(),
                dest_folder: destination_folder,
                rule_id,
                conflict_policy: rule.conflict_policy.clone(),
                activation,
                expected_snapshot,
                is_undo: false,
                undo_exemptions: self.undo_exemptions.clone(),
                in_flight: in_flight_guard,
                app_state_db: Arc::clone(self.app_state_db),
                shutdown: Arc::clone(self.shutdown),
            })
            .is_err()
        {
            let message = rust_i18n::t!("organizer.error_file_worker_unavailable").to_string();
            if let Err(error) = self.app_state_db.finish_organizer_operation(
                new_operation_id,
                OrganizerOperationStatus::Failed,
                Some(&message),
            ) {
                log::error!(
                    "[ORGANIZER] Failed to finalize undispatched retry {}: {}",
                    new_operation_id,
                    error
                );
            }
            return self.respond(
                command_id,
                Err(OrganizerCommandError::OperationDispatchFailed { reason: message }),
            );
        }
        self.respond(
            command_id,
            Ok(OrganizerCommandResult::RetryQueued {
                operation_id: new_operation_id,
                original_operation_id: operation_id,
            }),
        )
    }

    fn undo_operation(
        &mut self,
        command_id: OrganizerCommandId,
        operation_id: OrganizerOperationId,
    ) -> bool {
        let record = match self.app_state_db.get_organizer_operation(operation_id) {
            Ok(Some(record)) => record,
            Ok(None) => {
                return self.respond(command_id, Err(OrganizerCommandError::UndoUnavailable))
            }
            Err(error) => {
                return self.respond(
                    command_id,
                    Err(OrganizerCommandError::OperationDispatchFailed {
                        reason: error.to_string(),
                    }),
                )
            }
        };
        if record.status != OrganizerOperationStatus::Completed {
            return self.respond(command_id, Err(OrganizerCommandError::UndoUnavailable));
        }
        let (Some(source), Some(destination), Some(expected_snapshot)) = (
            record.effective_destination_path.clone(),
            record.effective_source_path.clone(),
            record.destination_snapshot_after,
        ) else {
            return self.respond(command_id, Err(OrganizerCommandError::UndoUnavailable));
        };
        let Ok(source) = safe_organizer_path(&source) else {
            return self.respond(command_id, Err(OrganizerCommandError::UndoUnavailable));
        };
        let Ok(destination) = safe_organizer_path(&destination) else {
            return self.respond(command_id, Err(OrganizerCommandError::UndoUnavailable));
        };
        if !source.is_file()
            || organizer_file_snapshot(&source).ok() != Some(expected_snapshot)
            || destination.try_exists().unwrap_or(true)
        {
            return self.respond(command_id, Err(OrganizerCommandError::UndoUnavailable));
        }
        let Some(destination_folder) = destination.parent().map(PathBuf::from) else {
            return self.respond(command_id, Err(OrganizerCommandError::UndoUnavailable));
        };
        if !destination_folder.is_dir() {
            return self.respond(command_id, Err(OrganizerCommandError::UndoUnavailable));
        }
        let source_key = super::normalize_watched_path(&source);
        let destination_key = super::normalize_watched_path(&destination);
        let Some(in_flight_guard) = self
            .in_flight
            .try_acquire_many([source_key, destination_key])
        else {
            return self.respond(command_id, Err(OrganizerCommandError::UndoUnavailable));
        };
        let new_operation_id = match self.app_state_db.create_undo_organizer_operation(
            operation_id,
            &source,
            &destination,
            expected_snapshot,
        ) {
            Ok(new_operation_id) => new_operation_id,
            Err(error) => {
                return self.respond(command_id, Err(undo_command_error(error)));
            }
        };
        let activation = Arc::new(AtomicBool::new(true));
        if self
            .file_operation_sender
            .send(FileOperationRequest::OrganizerMove {
                operation_id: new_operation_id,
                path: source,
                dest_folder: destination_folder,
                rule_id: record.rule_id.unwrap_or_default(),
                conflict_policy: OrganizerConflictPolicy::Ask,
                activation,
                expected_snapshot,
                is_undo: true,
                undo_exemptions: self.undo_exemptions.clone(),
                in_flight: in_flight_guard,
                app_state_db: Arc::clone(self.app_state_db),
                shutdown: Arc::clone(self.shutdown),
            })
            .is_err()
        {
            let message = rust_i18n::t!("organizer.error_file_worker_unavailable").to_string();
            if let Err(error) = self.app_state_db.finish_organizer_operation(
                new_operation_id,
                OrganizerOperationStatus::Failed,
                Some(&message),
            ) {
                log::error!(
                    "[ORGANIZER] Failed to finalize undispatched undo {}: {}",
                    new_operation_id,
                    error
                );
            }
            return self.respond(
                command_id,
                Err(OrganizerCommandError::OperationDispatchFailed { reason: message }),
            );
        }
        self.respond(
            command_id,
            Ok(OrganizerCommandResult::UndoQueued {
                operation_id: new_operation_id,
                original_operation_id: operation_id,
            }),
        )
    }

    fn cancel_conflict(
        &self,
        conflict_id: crate::domain::organizer_conflict::OrganizerConflictId,
    ) -> Result<(), OrganizerCommandError> {
        self.app_state_db
            .finish_organizer_conflict(conflict_id, OrganizerConflictStatus::Cancelled)
            .map_err(|error| OrganizerCommandError::ConflictResolutionFailed {
                reason: error.to_string(),
            })
    }

    fn rename_conflict_source(
        &mut self,
        conflict: &OrganizerConflictRecord,
        new_name: &str,
    ) -> Result<(PathBuf, PathBuf), OrganizerCommandError> {
        validate_organizer_conflict_name(new_name)
            .map_err(|_| OrganizerCommandError::InvalidConflictName)?;
        let parent = conflict
            .source_path
            .parent()
            .map(PathBuf::from)
            .ok_or(OrganizerCommandError::ConflictStale)?;
        let target = parent.join(new_name);
        if target == conflict.source_path || target.exists() {
            return Err(OrganizerCommandError::ConflictTargetExists);
        }
        let target = safe_organizer_path(&target)?;
        self.claim_conflict_resolution(conflict.conflict_id, true, &target)?;
        let (source, destination, source_snapshot, destination_snapshot) =
            match self.current_conflict_state(conflict) {
                Ok(state) => state,
                Err(error) => {
                    let _ = self
                        .app_state_db
                        .release_organizer_conflict_resolution(conflict.conflict_id);
                    return Err(error);
                }
            };
        let reconciled = if let Err(error) = move_organizer_file_without_replace_guarded_by(
            &source,
            &target,
            source_snapshot,
            &destination,
            destination_snapshot,
        ) {
            self.reconcile_conflict_rename_error(conflict.conflict_id, error)?
        } else {
            false
        };

        if !reconciled {
            self.finish_conflict_after_rename(conflict.conflict_id)?;
        }
        queue_matching_path(
            self.rules,
            self.activation_flags,
            self.paused_rules,
            target.clone(),
            self.pending,
        );
        Ok((source, target))
    }

    fn rename_conflict_destination(
        &mut self,
        conflict: &OrganizerConflictRecord,
        new_name: &str,
    ) -> Result<(PathBuf, PathBuf), OrganizerCommandError> {
        validate_organizer_conflict_name(new_name)
            .map_err(|_| OrganizerCommandError::InvalidConflictName)?;
        let parent = conflict
            .destination_path
            .parent()
            .map(PathBuf::from)
            .ok_or(OrganizerCommandError::ConflictStale)?;
        let target = parent.join(new_name);
        if target == conflict.destination_path || target.exists() {
            return Err(OrganizerCommandError::ConflictTargetExists);
        }
        let target = safe_organizer_path(&target)?;
        self.claim_conflict_resolution(conflict.conflict_id, false, &target)?;
        let (source, destination, source_snapshot, destination_snapshot) =
            match self.current_conflict_state(conflict) {
                Ok(state) => state,
                Err(error) => {
                    let _ = self
                        .app_state_db
                        .release_organizer_conflict_resolution(conflict.conflict_id);
                    return Err(error);
                }
            };
        let reconciled = if let Err(error) = move_organizer_file_without_replace_guarded_by(
            &destination,
            &target,
            destination_snapshot,
            &source,
            source_snapshot,
        ) {
            self.reconcile_conflict_rename_error(conflict.conflict_id, error)?
        } else {
            false
        };
        if !reconciled {
            self.finish_conflict_after_rename(conflict.conflict_id)?;
        }
        queue_matching_path(
            self.rules,
            self.activation_flags,
            self.paused_rules,
            source,
            self.pending,
        );
        Ok((destination, target))
    }

    fn current_conflict_state(
        &self,
        conflict: &OrganizerConflictRecord,
    ) -> Result<
        (
            PathBuf,
            PathBuf,
            OrganizerFileSnapshot,
            OrganizerFileSnapshot,
        ),
        OrganizerCommandError,
    > {
        let source = safe_organizer_path(&conflict.source_path)?;
        let destination = safe_organizer_path(&conflict.destination_path)?;
        if !source.is_file() || !destination.exists() {
            return Err(self.mark_conflict_obsolete(conflict.conflict_id));
        }
        let source_snapshot = organizer_file_snapshot(&source)
            .map_err(|_| self.mark_conflict_obsolete(conflict.conflict_id))?;
        if source_snapshot != conflict.source_snapshot {
            return Err(self.mark_conflict_obsolete(conflict.conflict_id));
        }
        let Some(expected_destination_snapshot) = conflict.destination_snapshot else {
            return Err(OrganizerCommandError::ConflictUnavailable);
        };
        let destination_snapshot = organizer_file_snapshot(&destination)
            .map_err(|_| self.mark_conflict_obsolete(conflict.conflict_id))?;
        if destination_snapshot != expected_destination_snapshot {
            return Err(self.mark_conflict_obsolete(conflict.conflict_id));
        }
        Ok((source, destination, source_snapshot, destination_snapshot))
    }

    fn finish_conflict_after_rename(
        &self,
        conflict_id: crate::domain::organizer_conflict::OrganizerConflictId,
    ) -> Result<(), OrganizerCommandError> {
        if let Err(error) = self
            .app_state_db
            .finish_organizer_conflict(conflict_id, OrganizerConflictStatus::Resolved)
        {
            if self
                .app_state_db
                .reconcile_owned_organizer_conflict_resolution(conflict_id)
                .is_ok()
                && self.conflict_status(conflict_id) == Some(OrganizerConflictStatus::Resolved)
            {
                return Ok(());
            }
            return Err(OrganizerCommandError::ConflictResolutionFailed {
                reason: error.to_string(),
            });
        }
        Ok(())
    }

    fn mark_conflict_obsolete(
        &self,
        conflict_id: crate::domain::organizer_conflict::OrganizerConflictId,
    ) -> OrganizerCommandError {
        match self
            .app_state_db
            .finish_organizer_conflict(conflict_id, OrganizerConflictStatus::Obsolete)
        {
            Ok(()) => OrganizerCommandError::ConflictStale,
            Err(error) => OrganizerCommandError::ConflictResolutionFailed {
                reason: error.to_string(),
            },
        }
    }

    fn claim_conflict_resolution(
        &self,
        conflict_id: crate::domain::organizer_conflict::OrganizerConflictId,
        rename_source: bool,
        target: &std::path::Path,
    ) -> Result<(), OrganizerCommandError> {
        self.app_state_db
            .claim_organizer_conflict_resolution(conflict_id, rename_source, target)
            .map_err(|error| match error {
                crate::infrastructure::app_state_db::OrganizerConflictDbError::AlreadyFinalized(_)
                | crate::infrastructure::app_state_db::OrganizerConflictDbError::NotFound(_)
                | crate::infrastructure::app_state_db::OrganizerConflictDbError::ResolutionInProgress(_) => {
                    OrganizerCommandError::ConflictUnavailable
                }
                error => OrganizerCommandError::ConflictResolutionFailed {
                    reason: error.to_string(),
                },
            })
    }

    fn reconcile_conflict_rename_error(
        &self,
        conflict_id: crate::domain::organizer_conflict::OrganizerConflictId,
        error: std::io::Error,
    ) -> Result<bool, OrganizerCommandError> {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            let _ = self
                .app_state_db
                .release_organizer_conflict_resolution(conflict_id);
            Err(OrganizerCommandError::ConflictTargetExists)
        } else if error.kind() == std::io::ErrorKind::InvalidData {
            Err(self.mark_conflict_obsolete(conflict_id))
        } else {
            if let Err(reconcile_error) = self
                .app_state_db
                .reconcile_owned_organizer_conflict_resolution(conflict_id)
            {
                return Err(OrganizerCommandError::ConflictResolutionFailed {
                    reason: format!("{error}; reconciliation failed: {reconcile_error}"),
                });
            }
            match self.conflict_status(conflict_id) {
                Some(OrganizerConflictStatus::Resolved) => Ok(true),
                Some(OrganizerConflictStatus::Obsolete) => {
                    Err(OrganizerCommandError::ConflictStale)
                }
                _ => Err(OrganizerCommandError::ConflictResolutionFailed {
                    reason: error.to_string(),
                }),
            }
        }
    }

    fn conflict_status(
        &self,
        conflict_id: crate::domain::organizer_conflict::OrganizerConflictId,
    ) -> Option<OrganizerConflictStatus> {
        self.app_state_db
            .get_organizer_conflict(conflict_id)
            .ok()
            .flatten()
            .map(|conflict| conflict.status)
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

fn retry_command_error(error: OrganizerOperationDbError) -> OrganizerCommandError {
    match error {
        OrganizerOperationDbError::NotFound(_) | OrganizerOperationDbError::RetryUnavailable(_) => {
            OrganizerCommandError::RetryUnavailable
        }
        error => OrganizerCommandError::OperationDispatchFailed {
            reason: error.to_string(),
        },
    }
}

fn undo_command_error(error: OrganizerOperationDbError) -> OrganizerCommandError {
    match error {
        OrganizerOperationDbError::NotFound(_) | OrganizerOperationDbError::UndoUnavailable(_) => {
            OrganizerCommandError::UndoUnavailable
        }
        error => OrganizerCommandError::OperationDispatchFailed {
            reason: error.to_string(),
        },
    }
}

fn safe_organizer_path(path: &std::path::Path) -> Result<PathBuf, OrganizerCommandError> {
    crate::workers::file_operation_worker::sanitize_organizer_path(path)
        .map_err(|_| OrganizerCommandError::SecurityViolation)
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
