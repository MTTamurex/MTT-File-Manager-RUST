use super::organizer_conflicts::{
    conflict_id_text, now_unix_millis, parse_conflict_id, parse_operation_id,
    OrganizerConflictDbError, OrganizerConflictRegistration,
};
use super::organizer_operations::{malformed_column, operation_id_text, path_bytes, path_text};
use super::AppStateDb;
use crate::domain::organizer_conflict::{OrganizerConflictId, OrganizerConflictStatus};
use crate::domain::organizer_operation::{OrganizerOperationId, OrganizerOperationStatus};
use crate::infrastructure::windows::shell_operations::OrganizerFileSnapshot;
use rusqlite::{params, OptionalExtension};
use std::path::Path;

fn reserve_conflict_id(
    tx: &rusqlite::Transaction<'_>,
) -> Result<OrganizerConflictId, OrganizerConflictDbError> {
    let next_id = tx
        .query_row(
            "UPDATE organizer_conflict_sequence
             SET next_id = next_id + 1
             WHERE singleton = 1 AND next_id < 9223372036854775807
             RETURNING next_id - 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .ok_or(OrganizerConflictDbError::IdExhausted)?;
    OrganizerConflictId::from_raw(next_id as u64).ok_or(OrganizerConflictDbError::IdExhausted)
}

fn reserve_operation_id(
    tx: &rusqlite::Transaction<'_>,
) -> Result<OrganizerOperationId, OrganizerConflictDbError> {
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
        .ok_or(OrganizerConflictDbError::IdExhausted)?;
    OrganizerOperationId::from_raw(next_id as u64).ok_or(OrganizerConflictDbError::IdExhausted)
}

fn operation_state(
    tx: &rusqlite::Transaction<'_>,
    operation_id: OrganizerOperationId,
) -> Result<(Option<String>, String), OrganizerConflictDbError> {
    tx.query_row(
        "SELECT conflict_id, status FROM organizer_operations WHERE operation_id = ?1",
        params![operation_id_text(operation_id)],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()?
    .ok_or(OrganizerConflictDbError::OperationNotFound(operation_id))
}

fn matching_conflict(
    tx: &rusqlite::Transaction<'_>,
    rule_id: i64,
    source_path: &Path,
    destination_path: &Path,
    source_snapshot: OrganizerFileSnapshot,
    destination_snapshot: Option<OrganizerFileSnapshot>,
) -> Result<
    Option<(
        OrganizerOperationId,
        OrganizerConflictId,
        OrganizerConflictStatus,
    )>,
    OrganizerConflictDbError,
> {
    tx.query_row(
        "SELECT operation_id, conflict_id, status
         FROM organizer_conflicts
         WHERE rule_id = ?1
           AND source_path = ?2 COLLATE NOCASE
           AND destination_path = ?3 COLLATE NOCASE
           AND source_snapshot = ?4
           AND destination_snapshot IS ?5
           AND status IN ('pending', 'cancelled')
         ORDER BY CASE status WHEN 'cancelled' THEN 0 ELSE 1 END, created_at DESC
         LIMIT 1",
        params![
            rule_id,
            path_text(source_path),
            path_text(destination_path),
            source_snapshot.to_bytes().to_vec(),
            destination_snapshot.map(|snapshot| snapshot.to_bytes().to_vec()),
        ],
        |row| {
            let operation_id = parse_operation_id(row.get(0)?, 0)?;
            let conflict_id = parse_conflict_id(row.get(1)?, 1)?;
            let status = OrganizerConflictStatus::from_persisted(&row.get::<_, String>(2)?)
                .ok_or_else(|| malformed_column(2, "invalid organizer conflict status"))?;
            Ok((operation_id, conflict_id, status))
        },
    )
    .optional()
    .map_err(OrganizerConflictDbError::from)
}

fn obsolete_previous_path_conflicts(
    tx: &rusqlite::Transaction<'_>,
    rule_id: i64,
    source_path: &Path,
    destination_path: &Path,
    checked_at: i64,
) -> Result<(), OrganizerConflictDbError> {
    tx.execute(
        "UPDATE organizer_conflicts
         SET status = 'obsolete', last_checked_at = ?1
         WHERE rule_id = ?2
           AND source_path = ?3 COLLATE NOCASE
           AND destination_path = ?4 COLLATE NOCASE
           AND status = 'pending'",
        params![
            checked_at,
            rule_id,
            path_text(source_path),
            path_text(destination_path),
        ],
    )?;
    Ok(())
}

fn finish_started_operation_with_conflict(
    tx: &rusqlite::Transaction<'_>,
    operation_id: OrganizerOperationId,
    conflict_id: OrganizerConflictId,
    source_path: &Path,
    destination_path: &Path,
    finished_at: i64,
    status: OrganizerOperationStatus,
) -> Result<(), OrganizerConflictDbError> {
    let updated = tx.execute(
        "UPDATE organizer_operations
         SET conflict_id = ?1, status = ?2, finished_at = ?3, error = NULL,
             effective_source_path = ?4, effective_destination_path = ?5,
             effective_source_path_bytes = ?6, effective_destination_path_bytes = ?7
         WHERE operation_id = ?8 AND status = 'started' AND finished_at IS NULL",
        params![
            conflict_id_text(conflict_id),
            status.as_str(),
            finished_at,
            path_text(source_path),
            path_text(destination_path),
            path_bytes(source_path),
            path_bytes(destination_path),
            operation_id_text(operation_id),
        ],
    )?;
    if updated != 1 {
        return Err(OrganizerConflictDbError::OperationAlreadyFinalized(
            operation_id,
        ));
    }
    tx.execute(
        "DELETE FROM organizer_operation_completions WHERE operation_id = ?1",
        params![operation_id_text(operation_id)],
    )?;
    Ok(())
}

impl AppStateDb {
    pub fn create_organizer_conflict(
        &self,
        operation_id: OrganizerOperationId,
        rule_id: i64,
        source_path: &Path,
        destination_path: &Path,
        source_snapshot: OrganizerFileSnapshot,
        destination_snapshot: Option<OrganizerFileSnapshot>,
    ) -> Result<OrganizerConflictRegistration, OrganizerConflictDbError> {
        let checked_at = now_unix_millis()?;
        let mut db = self
            .writer
            .lock()
            .map_err(|_| OrganizerConflictDbError::DatabaseUnavailable)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (existing_conflict_id, operation_status) = operation_state(&tx, operation_id)?;
        if let Some(raw_id) = existing_conflict_id {
            let conflict_id = parse_conflict_id(raw_id, 0)?;
            return Ok(OrganizerConflictRegistration::Existing {
                operation_id,
                conflict_id,
            });
        }
        if operation_status != "started" {
            return Err(OrganizerConflictDbError::OperationAlreadyFinalized(
                operation_id,
            ));
        }

        tx.execute(
            "UPDATE organizer_operations
             SET source_snapshot_before = COALESCE(source_snapshot_before, ?1)
             WHERE operation_id = ?2",
            params![
                source_snapshot.to_bytes().to_vec(),
                operation_id_text(operation_id),
            ],
        )?;

        if let Some((_existing_operation_id, conflict_id, status)) = matching_conflict(
            &tx,
            rule_id,
            source_path,
            destination_path,
            source_snapshot,
            destination_snapshot,
        )? {
            if status == OrganizerConflictStatus::Cancelled {
                obsolete_previous_path_conflicts(
                    &tx,
                    rule_id,
                    source_path,
                    destination_path,
                    checked_at,
                )?;
                tx.execute(
                    "UPDATE organizer_conflicts SET last_checked_at = ?1 WHERE conflict_id = ?2",
                    params![checked_at, conflict_id_text(conflict_id)],
                )?;
                finish_started_operation_with_conflict(
                    &tx,
                    operation_id,
                    conflict_id,
                    source_path,
                    destination_path,
                    checked_at,
                    OrganizerOperationStatus::Cancelled,
                )?;
                tx.commit()?;
                return Ok(OrganizerConflictRegistration::Suppressed);
            }
            tx.execute(
                "UPDATE organizer_conflicts SET last_checked_at = ?1 WHERE conflict_id = ?2",
                params![checked_at, conflict_id_text(conflict_id)],
            )?;
            finish_started_operation_with_conflict(
                &tx,
                operation_id,
                conflict_id,
                source_path,
                destination_path,
                checked_at,
                OrganizerOperationStatus::Skipped,
            )?;
            tx.commit()?;
            return Ok(OrganizerConflictRegistration::Existing {
                operation_id,
                conflict_id,
            });
        }

        obsolete_previous_path_conflicts(&tx, rule_id, source_path, destination_path, checked_at)?;
        let conflict_id = reserve_conflict_id(&tx)?;
        tx.execute(
            "INSERT INTO organizer_conflicts
                (conflict_id, operation_id, rule_id, source_path, destination_path,
                 source_path_bytes, destination_path_bytes, source_snapshot,
                 destination_snapshot, created_at, last_checked_at, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, 'pending')",
            params![
                conflict_id_text(conflict_id),
                operation_id_text(operation_id),
                rule_id,
                path_text(source_path),
                path_text(destination_path),
                path_bytes(source_path),
                path_bytes(destination_path),
                source_snapshot.to_bytes().to_vec(),
                destination_snapshot.map(|snapshot| snapshot.to_bytes().to_vec()),
                checked_at,
            ],
        )?;
        finish_started_operation_with_conflict(
            &tx,
            operation_id,
            conflict_id,
            source_path,
            destination_path,
            checked_at,
            OrganizerOperationStatus::Skipped,
        )?;
        tx.commit()?;
        Ok(OrganizerConflictRegistration::Created {
            operation_id,
            conflict_id,
        })
    }

    pub fn record_terminal_organizer_conflict(
        &self,
        rule_id: i64,
        source_path: &Path,
        destination_path: &Path,
        source_snapshot: OrganizerFileSnapshot,
        destination_snapshot: Option<OrganizerFileSnapshot>,
    ) -> Result<OrganizerConflictRegistration, OrganizerConflictDbError> {
        let checked_at = now_unix_millis()?;
        let mut db = self
            .writer
            .lock()
            .map_err(|_| OrganizerConflictDbError::DatabaseUnavailable)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some((operation_id, conflict_id, status)) = matching_conflict(
            &tx,
            rule_id,
            source_path,
            destination_path,
            source_snapshot,
            destination_snapshot,
        )? {
            if status == OrganizerConflictStatus::Cancelled {
                obsolete_previous_path_conflicts(
                    &tx,
                    rule_id,
                    source_path,
                    destination_path,
                    checked_at,
                )?;
            }
            tx.execute(
                "UPDATE organizer_conflicts SET last_checked_at = ?1 WHERE conflict_id = ?2",
                params![checked_at, conflict_id_text(conflict_id)],
            )?;
            tx.commit()?;
            return if status == OrganizerConflictStatus::Cancelled {
                Ok(OrganizerConflictRegistration::Suppressed)
            } else {
                Ok(OrganizerConflictRegistration::Existing {
                    operation_id,
                    conflict_id,
                })
            };
        }

        obsolete_previous_path_conflicts(&tx, rule_id, source_path, destination_path, checked_at)?;
        let operation_id = reserve_operation_id(&tx)?;
        let conflict_id = reserve_conflict_id(&tx)?;
        tx.execute(
            "INSERT INTO organizer_operations
                (operation_id, rule_id, source_path, destination_path, operation_type, status,
                 created_at, started_at, finished_at, error, conflict_id,
                 effective_source_path, effective_destination_path, source_snapshot_before,
                 source_path_bytes, destination_path_bytes, effective_source_path_bytes,
                 effective_destination_path_bytes)
             VALUES (?1, ?2, ?3, ?4, 'move', 'skipped', ?5, ?5, ?5, NULL, ?6,
                     ?3, ?4, ?7, ?8, ?9, ?8, ?9)",
            params![
                operation_id_text(operation_id),
                rule_id,
                path_text(source_path),
                path_text(destination_path),
                checked_at,
                conflict_id_text(conflict_id),
                source_snapshot.to_bytes().to_vec(),
                path_bytes(source_path),
                path_bytes(destination_path),
            ],
        )?;
        tx.execute(
            "INSERT INTO organizer_conflicts
                (conflict_id, operation_id, rule_id, source_path, destination_path,
                 source_path_bytes, destination_path_bytes, source_snapshot,
                 destination_snapshot, created_at, last_checked_at, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, 'pending')",
            params![
                conflict_id_text(conflict_id),
                operation_id_text(operation_id),
                rule_id,
                path_text(source_path),
                path_text(destination_path),
                path_bytes(source_path),
                path_bytes(destination_path),
                source_snapshot.to_bytes().to_vec(),
                destination_snapshot.map(|snapshot| snapshot.to_bytes().to_vec()),
                checked_at,
            ],
        )?;
        tx.commit()?;
        Ok(OrganizerConflictRegistration::Created {
            operation_id,
            conflict_id,
        })
    }
}
