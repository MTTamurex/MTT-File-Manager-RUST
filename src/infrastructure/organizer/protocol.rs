use super::OrganizerCommandId;
use crate::domain::organizer_conflict::OrganizerConflictId;
use crate::domain::organizer_operation::OrganizerOperationId;
use crate::domain::organizer_rule::OrganizerRuleError;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OrganizerConflictResolution {
    RenameSource { new_name: String },
    RenameDestination { new_name: String },
    Cancel,
}

#[derive(Debug, Eq, PartialEq)]
pub enum OrganizerCommandResult {
    RulesUpdated {
        rule_count: usize,
    },
    RulePaused {
        rule_id: i64,
    },
    RuleResumed {
        rule_id: i64,
    },
    FolderReady {
        rule_id: i64,
        source: bool,
        path: std::path::PathBuf,
    },
    ConflictResolved {
        conflict_id: OrganizerConflictId,
        old_path: std::path::PathBuf,
        new_path: std::path::PathBuf,
    },
    ConflictCancelled {
        conflict_id: OrganizerConflictId,
    },
    RetryQueued {
        operation_id: OrganizerOperationId,
        original_operation_id: OrganizerOperationId,
    },
    UndoQueued {
        operation_id: OrganizerOperationId,
        original_operation_id: OrganizerOperationId,
    },
}

#[derive(Debug, Eq, PartialEq)]
pub enum OrganizerRetryUnavailableReason {
    OperationNotFound,
    OperationNotEligible,
    RuleNotFound,
    RuleDisabled,
    RulePaused,
    SourceMissing,
    SourceUnavailable,
    SourceChanged,
    SourceNoLongerMatchesRule,
    SourcePathInvalid,
    DestinationPathInvalid,
    DestinationMissing,
    DestinationUnavailable,
    OperationInProgress,
}

#[derive(Debug, Eq, PartialEq)]
pub enum OrganizerUndoUnavailableReason {
    OperationNotFound,
    OperationNotEligible,
    AlreadyApplied,
    OperationInProgress,
    SourcePathInvalid,
    SourceMissing,
    SourceUnavailable,
    SourceChanged,
    TargetPathInvalid,
    TargetMissing,
    TargetUnavailable,
    TargetOccupied,
}

#[derive(Debug, Eq, PartialEq)]
pub enum OrganizerCommandError {
    ManagerUnavailable,
    CommandIdExhausted,
    InvalidRules(OrganizerRuleError),
    DuplicateRuleId { rule_id: i64 },
    RuleUnavailable,
    SecurityViolation,
    FolderCreationFailed { reason: String },
    ConflictUnavailable,
    InvalidConflictName,
    ConflictStale,
    ConflictTargetExists,
    ConflictResolutionFailed { reason: String },
    RetryUnavailable(OrganizerRetryUnavailableReason),
    UndoUnavailable(OrganizerUndoUnavailableReason),
    OperationDispatchFailed { reason: String },
}

impl fmt::Display for OrganizerCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ManagerUnavailable => {
                formatter.write_str(rust_i18n::t!("organizer.error_manager_unavailable").as_ref())
            }
            Self::CommandIdExhausted => {
                formatter.write_str(rust_i18n::t!("organizer.error_command_id_exhausted").as_ref())
            }
            Self::InvalidRules(error) => {
                let message = match error {
                    OrganizerRuleError::InvalidExtensions => {
                        rust_i18n::t!("organizer.error_invalid_extensions")
                    }
                    OrganizerRuleError::MissingExtensions => {
                        rust_i18n::t!("organizer.error_missing_extensions")
                    }
                    OrganizerRuleError::RelativeFolder => {
                        rust_i18n::t!("organizer.error_relative_folder")
                    }
                    OrganizerRuleError::SourceFolderMissing => {
                        rust_i18n::t!("organizer.error_source_missing")
                    }
                    OrganizerRuleError::DestinationFolderMissing => {
                        rust_i18n::t!("organizer.error_destination_missing")
                    }
                    OrganizerRuleError::SameFolders => {
                        rust_i18n::t!("organizer.error_same_folders")
                    }
                    OrganizerRuleError::InvalidConflictFolder => {
                        rust_i18n::t!("organizer.error_invalid_conflict_folder")
                    }
                    OrganizerRuleError::RuleCycle => rust_i18n::t!("organizer.error_rule_cycle"),
                };
                formatter.write_str(message.as_ref())
            }
            Self::DuplicateRuleId { rule_id } => write!(
                formatter,
                "{}",
                rust_i18n::t!("organizer.error_duplicate_rule_id", rule_id = rule_id)
            ),
            Self::RuleUnavailable => {
                formatter.write_str(rust_i18n::t!("organizer.error_rule_unavailable").as_ref())
            }
            Self::SecurityViolation => {
                formatter.write_str(rust_i18n::t!("organizer.error_security_path").as_ref())
            }
            Self::FolderCreationFailed { reason } => {
                write!(
                    formatter,
                    "{}",
                    rust_i18n::t!("organizer.error_create_folder", reason = reason)
                )
            }
            Self::ConflictUnavailable => {
                formatter.write_str(rust_i18n::t!("organizer.error_conflict_unavailable").as_ref())
            }
            Self::InvalidConflictName => {
                formatter.write_str(rust_i18n::t!("organizer.error_invalid_conflict_name").as_ref())
            }
            Self::ConflictStale => {
                formatter.write_str(rust_i18n::t!("organizer.error_conflict_stale").as_ref())
            }
            Self::ConflictTargetExists => formatter
                .write_str(rust_i18n::t!("organizer.error_conflict_target_exists").as_ref()),
            Self::ConflictResolutionFailed { reason } => {
                write!(
                    formatter,
                    "{}",
                    rust_i18n::t!("organizer.error_conflict_resolution", reason = reason)
                )
            }
            Self::RetryUnavailable(reason) => {
                let message = match reason {
                    OrganizerRetryUnavailableReason::OperationNotFound => {
                        rust_i18n::t!("organizer.error_retry_operation_missing")
                    }
                    OrganizerRetryUnavailableReason::OperationNotEligible => {
                        rust_i18n::t!("organizer.error_retry_operation_not_eligible")
                    }
                    OrganizerRetryUnavailableReason::RuleNotFound => {
                        rust_i18n::t!("organizer.error_retry_rule_missing")
                    }
                    OrganizerRetryUnavailableReason::RuleDisabled => {
                        rust_i18n::t!("organizer.error_retry_rule_disabled")
                    }
                    OrganizerRetryUnavailableReason::RulePaused => {
                        rust_i18n::t!("organizer.error_retry_rule_paused")
                    }
                    OrganizerRetryUnavailableReason::SourceMissing => {
                        rust_i18n::t!("organizer.error_retry_source_missing")
                    }
                    OrganizerRetryUnavailableReason::SourceUnavailable => {
                        rust_i18n::t!("organizer.error_retry_source_unavailable")
                    }
                    OrganizerRetryUnavailableReason::SourceChanged => {
                        rust_i18n::t!("organizer.error_retry_source_changed")
                    }
                    OrganizerRetryUnavailableReason::SourceNoLongerMatchesRule => {
                        rust_i18n::t!("organizer.error_retry_source_no_longer_matches")
                    }
                    OrganizerRetryUnavailableReason::SourcePathInvalid => {
                        rust_i18n::t!("organizer.error_retry_source_path_invalid")
                    }
                    OrganizerRetryUnavailableReason::DestinationPathInvalid => {
                        rust_i18n::t!("organizer.error_retry_destination_path_invalid")
                    }
                    OrganizerRetryUnavailableReason::DestinationMissing => {
                        rust_i18n::t!("organizer.error_retry_destination_missing")
                    }
                    OrganizerRetryUnavailableReason::DestinationUnavailable => {
                        rust_i18n::t!("organizer.error_retry_destination_unavailable")
                    }
                    OrganizerRetryUnavailableReason::OperationInProgress => {
                        rust_i18n::t!("organizer.error_retry_in_progress")
                    }
                };
                formatter.write_str(message.as_ref())
            }
            Self::UndoUnavailable(reason) => {
                let message = match reason {
                    OrganizerUndoUnavailableReason::OperationNotFound => {
                        rust_i18n::t!("organizer.error_undo_operation_missing")
                    }
                    OrganizerUndoUnavailableReason::OperationNotEligible => {
                        rust_i18n::t!("organizer.error_undo_operation_not_eligible")
                    }
                    OrganizerUndoUnavailableReason::AlreadyApplied => {
                        rust_i18n::t!("organizer.error_undo_already_applied")
                    }
                    OrganizerUndoUnavailableReason::OperationInProgress => {
                        rust_i18n::t!("organizer.error_undo_in_progress")
                    }
                    OrganizerUndoUnavailableReason::SourcePathInvalid => {
                        rust_i18n::t!("organizer.error_undo_source_path_invalid")
                    }
                    OrganizerUndoUnavailableReason::SourceMissing => {
                        rust_i18n::t!("organizer.error_undo_source_missing")
                    }
                    OrganizerUndoUnavailableReason::SourceUnavailable => {
                        rust_i18n::t!("organizer.error_undo_source_unavailable")
                    }
                    OrganizerUndoUnavailableReason::SourceChanged => {
                        rust_i18n::t!("organizer.error_undo_source_changed")
                    }
                    OrganizerUndoUnavailableReason::TargetPathInvalid => {
                        rust_i18n::t!("organizer.error_undo_target_path_invalid")
                    }
                    OrganizerUndoUnavailableReason::TargetMissing => {
                        rust_i18n::t!("organizer.error_undo_target_missing")
                    }
                    OrganizerUndoUnavailableReason::TargetUnavailable => {
                        rust_i18n::t!("organizer.error_undo_target_unavailable")
                    }
                    OrganizerUndoUnavailableReason::TargetOccupied => {
                        rust_i18n::t!("organizer.error_undo_target_exists")
                    }
                };
                formatter.write_str(message.as_ref())
            }
            Self::OperationDispatchFailed { reason } => write!(
                formatter,
                "{}",
                rust_i18n::t!("organizer.error_operation_dispatch", reason = reason)
            ),
        }
    }
}

impl std::error::Error for OrganizerCommandError {}

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
    CommandResult {
        command_id: OrganizerCommandId,
        result: Result<OrganizerCommandResult, OrganizerCommandError>,
    },
    Status {
        rule_id: i64,
        status: OrganizerRuleStatus,
    },
    OperationQueued {
        operation_id: OrganizerOperationId,
    },
    OperationSkipped {
        operation_id: OrganizerOperationId,
        conflict_id: Option<OrganizerConflictId>,
        rule_id: i64,
        path: std::path::PathBuf,
        destination: std::path::PathBuf,
    },
    OperationFailed {
        operation_id: OrganizerOperationId,
        rule_id: i64,
        path: std::path::PathBuf,
        destination: std::path::PathBuf,
        message: String,
    },
    Error {
        message: String,
    },
}
