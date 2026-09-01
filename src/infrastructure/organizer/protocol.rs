use super::OrganizerCommandId;
use crate::domain::organizer_operation::OrganizerOperationId;
use crate::domain::organizer_rule::OrganizerRuleError;
use std::fmt;

#[derive(Debug, Eq, PartialEq)]
pub enum OrganizerCommandResult {
    RulesUpdated {
        rule_count: usize,
    },
    RuleRunQueued {
        rule_id: i64,
    },
    RulePaused {
        rule_id: i64,
    },
    RuleResumed {
        rule_id: i64,
    },
    RefreshQueued {
        enabled_rule_count: usize,
    },
    FolderReady {
        rule_id: i64,
        source: bool,
        path: std::path::PathBuf,
    },
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
    OperationSkipped {
        operation_id: OrganizerOperationId,
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
