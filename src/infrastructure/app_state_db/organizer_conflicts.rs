use super::organizer_operations::{malformed_column, path_from_storage};
use super::AppStateDb;
use crate::domain::organizer_conflict::{OrganizerConflictId, OrganizerConflictStatus};
use crate::domain::organizer_operation::OrganizerOperationId;
use crate::infrastructure::windows::shell_operations::OrganizerFileSnapshot;
use rusqlite::{params, OptionalExtension, Row};
use std::path::PathBuf;
use std::sync::MutexGuard;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, thiserror::Error)]
pub enum OrganizerConflictDbError {
    #[error("app-state database is unavailable")]
    DatabaseUnavailable,
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error("the system clock is unavailable")]
    ClockUnavailable,
    #[error("organizer conflict IDs are exhausted")]
    IdExhausted,
    #[error("organizer operation {0} was not found")]
    OperationNotFound(OrganizerOperationId),
    #[error("organizer operation {0} is already finalized")]
    OperationAlreadyFinalized(OrganizerOperationId),
    #[error("organizer conflict {0} was not found")]
    NotFound(OrganizerConflictId),
    #[error("organizer conflict {0} is already finalized")]
    AlreadyFinalized(OrganizerConflictId),
    #[error("organizer conflict {0} is already being resolved")]
    ResolutionInProgress(OrganizerConflictId),
    #[error("organizer conflict status must be terminal")]
    InvalidStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrganizerConflictRecord {
    pub conflict_id: OrganizerConflictId,
    pub operation_id: OrganizerOperationId,
    pub rule_id: i64,
    pub source_path: PathBuf,
    pub destination_path: PathBuf,
    pub source_snapshot: OrganizerFileSnapshot,
    pub destination_snapshot: Option<OrganizerFileSnapshot>,
    pub created_at: i64,
    pub last_checked_at: i64,
    pub status: OrganizerConflictStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrganizerConflictRegistration {
    Created {
        operation_id: OrganizerOperationId,
        conflict_id: OrganizerConflictId,
    },
    Existing {
        operation_id: OrganizerOperationId,
        conflict_id: OrganizerConflictId,
    },
    Suppressed,
}

pub(super) fn now_unix_millis() -> Result<i64, OrganizerConflictDbError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| OrganizerConflictDbError::ClockUnavailable)?
        .as_millis();
    i64::try_from(millis).map_err(|_| OrganizerConflictDbError::ClockUnavailable)
}

pub(super) fn conflict_id_text(conflict_id: OrganizerConflictId) -> String {
    conflict_id.to_string()
}

fn malformed_blob(column: usize, message: &'static str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Blob,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message,
        )),
    )
}

pub(super) fn parse_operation_id(
    raw_id: String,
    column: usize,
) -> rusqlite::Result<OrganizerOperationId> {
    raw_id
        .parse::<u64>()
        .ok()
        .and_then(OrganizerOperationId::from_raw)
        .ok_or_else(|| malformed_column(column, "invalid organizer operation ID"))
}

pub(super) fn parse_conflict_id(
    raw_id: String,
    column: usize,
) -> rusqlite::Result<OrganizerConflictId> {
    raw_id
        .parse::<u64>()
        .ok()
        .and_then(OrganizerConflictId::from_raw)
        .ok_or_else(|| malformed_column(column, "invalid organizer conflict ID"))
}

pub(super) fn snapshot_from_storage(
    bytes: Vec<u8>,
    column: usize,
) -> rusqlite::Result<OrganizerFileSnapshot> {
    OrganizerFileSnapshot::from_bytes(&bytes)
        .ok_or_else(|| malformed_blob(column, "invalid organizer conflict snapshot"))
}

pub(super) fn optional_snapshot_from_storage(
    bytes: Option<Vec<u8>>,
    column: usize,
) -> rusqlite::Result<Option<OrganizerFileSnapshot>> {
    bytes
        .map(|bytes| snapshot_from_storage(bytes, column))
        .transpose()
}

fn record_from_row(row: &Row<'_>) -> rusqlite::Result<OrganizerConflictRecord> {
    let conflict_id = parse_conflict_id(row.get(0)?, 0)?;
    let operation_id = parse_operation_id(row.get(1)?, 1)?;
    let status_text = row.get::<_, String>(9)?;
    let status = OrganizerConflictStatus::from_persisted(&status_text)
        .ok_or_else(|| malformed_column(9, "invalid organizer conflict status"))?;
    Ok(OrganizerConflictRecord {
        conflict_id,
        operation_id,
        rule_id: row.get(2)?,
        source_path: path_from_storage(row.get(3)?, row.get(10)?, 10)?,
        destination_path: path_from_storage(row.get(4)?, row.get(11)?, 11)?,
        source_snapshot: snapshot_from_storage(row.get(5)?, 5)?,
        destination_snapshot: optional_snapshot_from_storage(row.get(6)?, 6)?,
        created_at: row.get(7)?,
        last_checked_at: row.get(8)?,
        status,
    })
}

fn writer<'a>(
    db: &'a AppStateDb,
) -> Result<MutexGuard<'a, rusqlite::Connection>, OrganizerConflictDbError> {
    db.writer
        .lock()
        .map_err(|_| OrganizerConflictDbError::DatabaseUnavailable)
}

impl AppStateDb {
    pub fn get_organizer_conflict(
        &self,
        conflict_id: OrganizerConflictId,
    ) -> Result<Option<OrganizerConflictRecord>, OrganizerConflictDbError> {
        let db = self
            .reader
            .lock()
            .map_err(|_| OrganizerConflictDbError::DatabaseUnavailable)?;
        db.query_row(
            "SELECT conflict_id, operation_id, rule_id, source_path, destination_path,
                    source_snapshot, destination_snapshot, created_at, last_checked_at, status,
                    source_path_bytes, destination_path_bytes
             FROM organizer_conflicts WHERE conflict_id = ?1",
            params![conflict_id_text(conflict_id)],
            record_from_row,
        )
        .optional()
        .map_err(OrganizerConflictDbError::from)
    }

    pub fn list_organizer_conflicts(
        &self,
        limit: usize,
    ) -> Result<Vec<OrganizerConflictRecord>, OrganizerConflictDbError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let db = self
            .reader
            .lock()
            .map_err(|_| OrganizerConflictDbError::DatabaseUnavailable)?;
        let mut statement = db.prepare(
            "SELECT conflict_id, operation_id, rule_id, source_path, destination_path,
                    source_snapshot, destination_snapshot, created_at, last_checked_at, status,
                    source_path_bytes, destination_path_bytes
             FROM organizer_conflicts
             ORDER BY created_at DESC, CAST(conflict_id AS INTEGER) DESC
             LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit], record_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(OrganizerConflictDbError::from)
    }

    pub fn list_pending_organizer_conflicts(
        &self,
        limit: usize,
    ) -> Result<Vec<OrganizerConflictRecord>, OrganizerConflictDbError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let db = self
            .reader
            .lock()
            .map_err(|_| OrganizerConflictDbError::DatabaseUnavailable)?;
        let mut statement = db.prepare(
            "SELECT conflict_id, operation_id, rule_id, source_path, destination_path,
                    source_snapshot, destination_snapshot, created_at, last_checked_at, status,
                    source_path_bytes, destination_path_bytes
             FROM organizer_conflicts
             WHERE status = 'pending'
             ORDER BY created_at DESC, CAST(conflict_id AS INTEGER) DESC
             LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit], record_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(OrganizerConflictDbError::from)
    }

    pub fn finish_organizer_conflict(
        &self,
        conflict_id: OrganizerConflictId,
        status: OrganizerConflictStatus,
    ) -> Result<(), OrganizerConflictDbError> {
        if !status.is_terminal() {
            return Err(OrganizerConflictDbError::InvalidStatus);
        }
        let checked_at = now_unix_millis()?;
        let mut db = writer(self)?;
        let tx = db.transaction()?;
        let updated = tx.execute(
            "UPDATE organizer_conflicts
             SET status = ?1, last_checked_at = ?2
             WHERE conflict_id = ?3 AND status = 'pending'
               AND (?1 <> 'cancelled' OR NOT EXISTS (
                   SELECT 1 FROM organizer_conflict_resolutions
                   WHERE conflict_id = ?3
               ))",
            params![status.as_str(), checked_at, conflict_id_text(conflict_id)],
        )?;
        if updated == 1 {
            tx.execute(
                "DELETE FROM organizer_conflict_resolutions WHERE conflict_id = ?1",
                params![conflict_id_text(conflict_id)],
            )?;
            tx.commit()?;
            return Ok(());
        }

        let existing_status = tx
            .query_row(
                "SELECT status FROM organizer_conflicts WHERE conflict_id = ?1",
                params![conflict_id_text(conflict_id)],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if existing_status.as_deref() == Some("pending")
            && tx.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM organizer_conflict_resolutions WHERE conflict_id = ?1
                 )",
                params![conflict_id_text(conflict_id)],
                |row| row.get::<_, bool>(0),
            )?
        {
            Err(OrganizerConflictDbError::ResolutionInProgress(conflict_id))
        } else if existing_status.is_some() {
            Err(OrganizerConflictDbError::AlreadyFinalized(conflict_id))
        } else {
            Err(OrganizerConflictDbError::NotFound(conflict_id))
        }
    }
}

#[cfg(test)]
#[path = "organizer_conflicts_tests.rs"]
mod tests;
