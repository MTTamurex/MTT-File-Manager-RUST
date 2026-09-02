use crate::domain::organizer_conflict::OrganizerConflictId;
use crate::domain::organizer_operation::{
    OrganizerOperationId, OrganizerOperationStatus, OrganizerOperationType,
};
use crate::infrastructure::windows::shell_operations::OrganizerFileSnapshot;
use rusqlite::{OptionalExtension, Row};
use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "organizer_operation_actions.rs"]
mod organizer_operation_actions;
#[path = "organizer_operation_history.rs"]
mod organizer_operation_history;

#[derive(Debug, thiserror::Error)]
pub enum OrganizerOperationDbError {
    #[error("app-state database is unavailable")]
    DatabaseUnavailable,
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error("the system clock is unavailable")]
    ClockUnavailable,
    #[error("organizer operation IDs are exhausted")]
    IdExhausted,
    #[error("organizer operation {0} was not found")]
    NotFound(OrganizerOperationId),
    #[error("organizer operation {0} is already finalized")]
    AlreadyFinalized(OrganizerOperationId),
    #[error("organizer operation status must be terminal")]
    InvalidStatus,
    #[error("organizer operation {0} cannot be retried")]
    RetryUnavailable(OrganizerOperationId),
    #[error("organizer operation {0} cannot be undone")]
    UndoUnavailable(OrganizerOperationId),
    #[error("organizer history retention must be between 1 and 3650 days")]
    InvalidRetention,
    #[error(transparent)]
    Filesystem(#[from] std::io::Error),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrganizerOperationRecord {
    pub operation_id: OrganizerOperationId,
    pub rule_id: Option<i64>,
    pub source_path: PathBuf,
    pub destination_path: PathBuf,
    pub operation_type: OrganizerOperationType,
    pub status: OrganizerOperationStatus,
    pub created_at: i64,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub error: Option<String>,
    pub conflict_id: Option<OrganizerConflictId>,
    pub original_operation_id: Option<OrganizerOperationId>,
    pub effective_source_path: Option<PathBuf>,
    pub effective_destination_path: Option<PathBuf>,
    pub source_snapshot_before: Option<OrganizerFileSnapshot>,
    pub destination_snapshot_after: Option<OrganizerFileSnapshot>,
    pub undone_at: Option<i64>,
}

pub(super) fn now_unix_millis() -> Result<i64, OrganizerOperationDbError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| OrganizerOperationDbError::ClockUnavailable)?
        .as_millis();
    i64::try_from(millis).map_err(|_| OrganizerOperationDbError::ClockUnavailable)
}

pub(super) fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub(super) fn path_bytes(path: &Path) -> Vec<u8> {
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

pub(super) fn operation_id_text(operation_id: OrganizerOperationId) -> String {
    operation_id.to_string()
}

pub const DEFAULT_ORGANIZER_HISTORY_RETENTION_DAYS: i64 = 30;
pub const MIN_ORGANIZER_HISTORY_RETENTION_DAYS: i64 = 1;
pub const MAX_ORGANIZER_HISTORY_RETENTION_DAYS: i64 = 3650;
pub(super) const ORGANIZER_HISTORY_RETENTION_PREFERENCE: &str = "organizer_history_retention_days";

pub(super) fn malformed_column(column: usize, message: &'static str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message,
        )),
    )
}

pub(super) fn path_from_storage(
    text: String,
    bytes: Option<Vec<u8>>,
    column: usize,
) -> rusqlite::Result<PathBuf> {
    let Some(bytes) = bytes else {
        return Ok(PathBuf::from(text));
    };
    if bytes.len() % std::mem::size_of::<u16>() != 0 {
        return Err(malformed_column(column, "invalid organizer operation path"));
    }
    let wide = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    Ok(PathBuf::from(OsString::from_wide(&wide)))
}

pub(super) fn record_from_row(row: &Row<'_>) -> rusqlite::Result<OrganizerOperationRecord> {
    let raw_id = row.get::<_, String>(0)?;
    let operation_id = raw_id
        .parse::<u64>()
        .ok()
        .and_then(OrganizerOperationId::from_raw)
        .ok_or_else(|| malformed_column(0, "invalid organizer operation ID"))?;
    let operation_type = OrganizerOperationType::from_persisted(&row.get::<_, String>(4)?)
        .ok_or_else(|| malformed_column(4, "invalid organizer operation type"))?;
    let raw_status = row.get::<_, String>(5)?;
    let status = OrganizerOperationStatus::from_persisted(&raw_status)
        .ok_or_else(|| malformed_column(5, "invalid organizer operation status"))?;
    let finished_at = row.get::<_, Option<i64>>(8)?;
    if status.is_terminal() != finished_at.is_some() {
        return Err(malformed_column(8, "invalid organizer operation lifecycle"));
    }
    let source_path = path_from_storage(
        row.get::<_, String>(2)?,
        row.get::<_, Option<Vec<u8>>>(16)?,
        16,
    )?;
    let destination_path = path_from_storage(
        row.get::<_, String>(3)?,
        row.get::<_, Option<Vec<u8>>>(17)?,
        17,
    )?;
    let conflict_id = row
        .get::<_, Option<String>>(10)?
        .map(|raw_id| {
            raw_id
                .parse::<u64>()
                .ok()
                .and_then(OrganizerConflictId::from_raw)
                .ok_or_else(|| malformed_column(10, "invalid organizer conflict ID"))
        })
        .transpose()?;
    let original_operation_id = row
        .get::<_, Option<String>>(11)?
        .map(|raw_id| {
            raw_id
                .parse::<u64>()
                .ok()
                .and_then(OrganizerOperationId::from_raw)
                .ok_or_else(|| malformed_column(11, "invalid original organizer operation ID"))
        })
        .transpose()?;
    let effective_source_path = optional_path_from_storage(
        row.get::<_, Option<String>>(12)?,
        row.get::<_, Option<Vec<u8>>>(18)?,
        18,
    )?;
    let effective_destination_path = optional_path_from_storage(
        row.get::<_, Option<String>>(13)?,
        row.get::<_, Option<Vec<u8>>>(19)?,
        19,
    )?;
    let source_snapshot_before = optional_snapshot_from_storage(row.get(14)?, 14)?;
    let destination_snapshot_after = optional_snapshot_from_storage(row.get(15)?, 15)?;

    Ok(OrganizerOperationRecord {
        operation_id,
        rule_id: row.get(1)?,
        source_path,
        destination_path,
        operation_type,
        status,
        created_at: row.get(6)?,
        started_at: row.get(7)?,
        finished_at,
        error: row.get(9)?,
        conflict_id,
        original_operation_id,
        effective_source_path,
        effective_destination_path,
        source_snapshot_before,
        destination_snapshot_after,
        undone_at: row.get(20)?,
    })
}

fn optional_path_from_storage(
    text: Option<String>,
    bytes: Option<Vec<u8>>,
    column: usize,
) -> rusqlite::Result<Option<PathBuf>> {
    match text {
        Some(text) => path_from_storage(text, bytes, column).map(Some),
        None if bytes.is_none() => Ok(None),
        None => Err(malformed_column(column, "invalid organizer operation path")),
    }
}

pub(super) fn malformed_blob(column: usize, message: &'static str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Blob,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message,
        )),
    )
}

fn optional_snapshot_from_storage(
    bytes: Option<Vec<u8>>,
    column: usize,
) -> rusqlite::Result<Option<OrganizerFileSnapshot>> {
    bytes
        .map(|bytes| {
            OrganizerFileSnapshot::from_bytes(&bytes)
                .ok_or_else(|| malformed_blob(column, "invalid organizer operation snapshot"))
        })
        .transpose()
}

pub(super) fn reserve_operation_id(
    tx: &rusqlite::Transaction<'_>,
) -> Result<OrganizerOperationId, OrganizerOperationDbError> {
    let next_id = tx
        .query_row(
            "UPDATE organizer_operation_sequence
             SET next_id = next_id + 1
             WHERE singleton = 1 AND next_id < 9223372036854775807
             RETURNING next_id - 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .ok_or(OrganizerOperationDbError::IdExhausted)?;
    OrganizerOperationId::from_raw(next_id as u64).ok_or(OrganizerOperationDbError::IdExhausted)
}

#[cfg(test)]
#[path = "organizer_operations_tests.rs"]
mod tests;
