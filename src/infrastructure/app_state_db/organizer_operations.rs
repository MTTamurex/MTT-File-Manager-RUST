use super::AppStateDb;
use crate::domain::organizer_operation::{OrganizerOperationId, OrganizerOperationStatus};
use rusqlite::{params, OptionalExtension, Row};
use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrganizerOperationRecord {
    pub operation_id: OrganizerOperationId,
    pub rule_id: i64,
    pub source_path: PathBuf,
    pub destination_path: PathBuf,
    pub status: OrganizerOperationStatus,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub error: Option<String>,
}

fn now_unix_millis() -> Result<i64, OrganizerOperationDbError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| OrganizerOperationDbError::ClockUnavailable)?
        .as_millis();
    i64::try_from(millis).map_err(|_| OrganizerOperationDbError::ClockUnavailable)
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn path_bytes(path: &Path) -> Vec<u8> {
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn operation_id_text(operation_id: OrganizerOperationId) -> String {
    operation_id.to_string()
}

fn malformed_column(column: usize, message: &'static str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message,
        )),
    )
}

fn path_from_storage(
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

fn record_from_row(row: &Row<'_>) -> rusqlite::Result<OrganizerOperationRecord> {
    let raw_id = row.get::<_, String>(0)?;
    let operation_id = raw_id
        .parse::<u64>()
        .ok()
        .and_then(OrganizerOperationId::from_raw)
        .ok_or_else(|| malformed_column(0, "invalid organizer operation ID"))?;
    let raw_status = row.get::<_, String>(4)?;
    let status = OrganizerOperationStatus::from_persisted(&raw_status)
        .ok_or_else(|| malformed_column(4, "invalid organizer operation status"))?;
    let finished_at = row.get::<_, Option<i64>>(6)?;
    if status.is_terminal() != finished_at.is_some() {
        return Err(malformed_column(6, "invalid organizer operation lifecycle"));
    }
    let source_path = path_from_storage(
        row.get::<_, String>(2)?,
        row.get::<_, Option<Vec<u8>>>(8)?,
        8,
    )?;
    let destination_path = path_from_storage(
        row.get::<_, String>(3)?,
        row.get::<_, Option<Vec<u8>>>(9)?,
        9,
    )?;

    Ok(OrganizerOperationRecord {
        operation_id,
        rule_id: row.get(1)?,
        source_path,
        destination_path,
        status,
        started_at: row.get(5)?,
        finished_at,
        error: row.get(7)?,
    })
}

fn reserve_operation_id(
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

impl AppStateDb {
    pub fn start_organizer_operation(
        &self,
        rule_id: i64,
        source_path: &Path,
        destination_path: &Path,
    ) -> Result<OrganizerOperationId, OrganizerOperationDbError> {
        let started_at = now_unix_millis()?;
        let mut db = self
            .writer
            .lock()
            .map_err(|_| OrganizerOperationDbError::DatabaseUnavailable)?;
        let tx = db.transaction()?;
        let operation_id = reserve_operation_id(&tx)?;
        tx.execute(
            "INSERT INTO organizer_operations
                (operation_id, rule_id, source_path, destination_path, status, started_at,
                 source_path_bytes, destination_path_bytes)
             VALUES (?1, ?2, ?3, ?4, 'started', ?5, ?6, ?7)",
            params![
                operation_id_text(operation_id),
                rule_id,
                path_text(source_path),
                path_text(destination_path),
                started_at,
                path_bytes(source_path),
                path_bytes(destination_path),
            ],
        )?;
        tx.commit()?;
        Ok(operation_id)
    }

    pub fn finish_organizer_operation(
        &self,
        operation_id: OrganizerOperationId,
        status: OrganizerOperationStatus,
        error: Option<&str>,
    ) -> Result<(), OrganizerOperationDbError> {
        if !status.is_terminal() {
            return Err(OrganizerOperationDbError::InvalidStatus);
        }

        let finished_at = now_unix_millis()?;
        let mut db = self
            .writer
            .lock()
            .map_err(|_| OrganizerOperationDbError::DatabaseUnavailable)?;
        let tx = db.transaction()?;
        let updated = tx.execute(
            "UPDATE organizer_operations
             SET status = ?1, finished_at = ?2, error = ?3
             WHERE operation_id = ?4 AND status = 'started' AND finished_at IS NULL",
            params![
                status.as_str(),
                finished_at,
                error,
                operation_id_text(operation_id)
            ],
        )?;
        if updated == 1 {
            tx.commit()?;
            return Ok(());
        }

        let existing_status = tx
            .query_row(
                "SELECT status FROM organizer_operations WHERE operation_id = ?1",
                params![operation_id_text(operation_id)],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if existing_status.is_some() {
            Err(OrganizerOperationDbError::AlreadyFinalized(operation_id))
        } else {
            Err(OrganizerOperationDbError::NotFound(operation_id))
        }
    }

    pub fn record_terminal_organizer_operation(
        &self,
        rule_id: i64,
        source_path: &Path,
        destination_path: &Path,
        status: OrganizerOperationStatus,
        error: Option<&str>,
    ) -> Result<OrganizerOperationId, OrganizerOperationDbError> {
        if !status.is_terminal() {
            return Err(OrganizerOperationDbError::InvalidStatus);
        }

        let finished_at = now_unix_millis()?;
        let mut db = self
            .writer
            .lock()
            .map_err(|_| OrganizerOperationDbError::DatabaseUnavailable)?;
        let tx = db.transaction()?;
        let operation_id = reserve_operation_id(&tx)?;
        tx.execute(
            "INSERT INTO organizer_operations
                (operation_id, rule_id, source_path, destination_path, status, started_at,
                 finished_at, error, source_path_bytes, destination_path_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8, ?9)",
            params![
                operation_id_text(operation_id),
                rule_id,
                path_text(source_path),
                path_text(destination_path),
                status.as_str(),
                finished_at,
                error,
                path_bytes(source_path),
                path_bytes(destination_path),
            ],
        )?;
        tx.commit()?;
        Ok(operation_id)
    }

    pub fn get_organizer_operation(
        &self,
        operation_id: OrganizerOperationId,
    ) -> Result<Option<OrganizerOperationRecord>, OrganizerOperationDbError> {
        let db = self
            .reader
            .lock()
            .map_err(|_| OrganizerOperationDbError::DatabaseUnavailable)?;
        db.query_row(
            "SELECT operation_id, rule_id, source_path, destination_path, status,
                    started_at, finished_at, error, source_path_bytes, destination_path_bytes
             FROM organizer_operations WHERE operation_id = ?1",
            params![operation_id_text(operation_id)],
            record_from_row,
        )
        .optional()
        .map_err(OrganizerOperationDbError::from)
    }

    pub fn list_organizer_operations(
        &self,
        limit: usize,
    ) -> Result<Vec<OrganizerOperationRecord>, OrganizerOperationDbError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let db = self
            .reader
            .lock()
            .map_err(|_| OrganizerOperationDbError::DatabaseUnavailable)?;
        let mut statement = db.prepare(
            "SELECT operation_id, rule_id, source_path, destination_path, status,
                    started_at, finished_at, error, source_path_bytes, destination_path_bytes
             FROM organizer_operations
             ORDER BY started_at DESC, CAST(operation_id AS INTEGER) DESC
             LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit], record_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(OrganizerOperationDbError::from)
    }
}

#[cfg(test)]
#[path = "organizer_operations_tests.rs"]
mod tests;
