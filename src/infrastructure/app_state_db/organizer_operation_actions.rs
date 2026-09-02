use super::{
    malformed_blob, now_unix_millis, operation_id_text, path_bytes, path_from_storage, path_text,
    reserve_operation_id, OrganizerOperationDbError,
};
use crate::domain::organizer_operation::{
    OrganizerOperationId, OrganizerOperationStatus, OrganizerOperationType,
};
use crate::infrastructure::app_state_db::{
    organizer_conflicts::parse_operation_id, process_owner_id, process_owner_is_active, AppStateDb,
};
use crate::infrastructure::windows::shell_operations::{
    organizer_file_snapshot, remove_organizer_file_if_matches, OrganizerFileSnapshot,
};
use rusqlite::{params, OptionalExtension};
use std::path::Path;

struct OrganizerOperationInsert<'a> {
    rule_id: Option<i64>,
    operation_type: OrganizerOperationType,
    source_path: &'a Path,
    destination_path: &'a Path,
    original_operation_id: Option<OrganizerOperationId>,
    source_snapshot_before: Option<OrganizerFileSnapshot>,
}

fn insert_started_operation(
    tx: &rusqlite::Transaction<'_>,
    started_at: i64,
    operation: OrganizerOperationInsert<'_>,
) -> Result<OrganizerOperationId, OrganizerOperationDbError> {
    let operation_id = reserve_operation_id(tx)?;
    let source_path_bytes = path_bytes(operation.source_path);
    let destination_path_bytes = path_bytes(operation.destination_path);
    tx.execute(
        "INSERT INTO organizer_operations
            (operation_id, rule_id, source_path, destination_path, operation_type, status,
             owner_id, created_at, started_at, original_operation_id, effective_source_path,
             effective_destination_path, source_snapshot_before, source_path_bytes,
             destination_path_bytes, effective_source_path_bytes, effective_destination_path_bytes)
         VALUES (?1, ?2, ?3, ?4, ?5, 'started', ?6, ?7, ?7, ?8, ?3, ?4, ?9, ?10, ?11, ?10, ?11)",
        params![
            operation_id_text(operation_id),
            operation.rule_id,
            path_text(operation.source_path),
            path_text(operation.destination_path),
            operation.operation_type.as_str(),
            process_owner_id(),
            started_at,
            operation.original_operation_id.map(operation_id_text),
            operation
                .source_snapshot_before
                .map(|snapshot| snapshot.to_bytes().to_vec()),
            source_path_bytes,
            destination_path_bytes,
        ],
    )?;
    Ok(operation_id)
}

impl AppStateDb {
    pub fn start_organizer_operation(
        &self,
        rule_id: i64,
        source_path: &Path,
        destination_path: &Path,
    ) -> Result<OrganizerOperationId, OrganizerOperationDbError> {
        self.start_organizer_operation_with_details(
            Some(rule_id),
            OrganizerOperationType::Move,
            source_path,
            destination_path,
            None,
            None,
        )
    }

    pub fn start_organizer_operation_with_snapshot(
        &self,
        rule_id: i64,
        source_path: &Path,
        destination_path: &Path,
        source_snapshot: OrganizerFileSnapshot,
    ) -> Result<OrganizerOperationId, OrganizerOperationDbError> {
        self.start_organizer_operation_with_details(
            Some(rule_id),
            OrganizerOperationType::Move,
            source_path,
            destination_path,
            None,
            Some(source_snapshot),
        )
    }

    fn start_organizer_operation_with_details(
        &self,
        rule_id: Option<i64>,
        operation_type: OrganizerOperationType,
        source_path: &Path,
        destination_path: &Path,
        original_operation_id: Option<OrganizerOperationId>,
        source_snapshot_before: Option<OrganizerFileSnapshot>,
    ) -> Result<OrganizerOperationId, OrganizerOperationDbError> {
        let started_at = now_unix_millis()?;
        let mut db = self
            .writer
            .lock()
            .map_err(|_| OrganizerOperationDbError::DatabaseUnavailable)?;
        let tx = db.transaction()?;
        let operation_id = insert_started_operation(
            &tx,
            started_at,
            OrganizerOperationInsert {
                rule_id,
                operation_type,
                source_path,
                destination_path,
                original_operation_id,
                source_snapshot_before,
            },
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
        self.finish_organizer_operation_with_metadata(operation_id, status, error, None, None, None)
    }

    pub fn finish_organizer_operation_with_metadata(
        &self,
        operation_id: OrganizerOperationId,
        status: OrganizerOperationStatus,
        error: Option<&str>,
        effective_source_path: Option<&Path>,
        effective_destination_path: Option<&Path>,
        destination_snapshot_after: Option<OrganizerFileSnapshot>,
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
             SET status = ?1, finished_at = ?2, error = ?3,
                 effective_source_path = COALESCE(?4, effective_source_path),
                 effective_destination_path = COALESCE(?5, effective_destination_path),
                 effective_source_path_bytes = COALESCE(?6, effective_source_path_bytes),
                 effective_destination_path_bytes = COALESCE(?7, effective_destination_path_bytes),
                 destination_snapshot_after = COALESCE(?8, destination_snapshot_after)
             WHERE operation_id = ?9 AND status = 'started' AND finished_at IS NULL",
            params![
                status.as_str(),
                finished_at,
                error,
                effective_source_path.map(path_text),
                effective_destination_path.map(path_text),
                effective_source_path.map(path_bytes),
                effective_destination_path.map(path_bytes),
                destination_snapshot_after.map(|snapshot| snapshot.to_bytes().to_vec()),
                operation_id_text(operation_id),
            ],
        )?;
        if updated == 1 {
            if status == OrganizerOperationStatus::Completed {
                tx.execute(
                    "UPDATE organizer_operations
                     SET undone_at = ?1
                     WHERE operation_id = (
                         SELECT original_operation_id FROM organizer_operations
                         WHERE operation_id = ?2 AND operation_type = 'undo'
                     )",
                    params![finished_at, operation_id_text(operation_id)],
                )?;
                tx.execute(
                    "INSERT OR REPLACE INTO organizer_undo_exemptions
                         (path_bytes, path, snapshot)
                     SELECT effective_destination_path_bytes, effective_destination_path,
                            destination_snapshot_after
                     FROM organizer_operations
                     WHERE operation_id = ?1 AND operation_type = 'undo'
                       AND effective_destination_path_bytes IS NOT NULL
                       AND effective_destination_path IS NOT NULL
                       AND destination_snapshot_after IS NOT NULL",
                    params![operation_id_text(operation_id)],
                )?;
                tx.execute(
                    "UPDATE organizer_conflicts
                     SET status = 'obsolete', last_checked_at = ?1
                     WHERE status = 'pending' AND EXISTS (
                         SELECT 1 FROM organizer_operations o
                         WHERE o.operation_id = ?2
                           AND o.rule_id = organizer_conflicts.rule_id
                           AND o.source_path = organizer_conflicts.source_path COLLATE NOCASE
                           AND o.destination_path = organizer_conflicts.destination_path COLLATE NOCASE
                     )",
                    params![finished_at, operation_id_text(operation_id)],
                )?;
            }
            tx.execute(
                "DELETE FROM organizer_operation_completions WHERE operation_id = ?1",
                params![operation_id_text(operation_id)],
            )?;
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
        self.record_terminal_organizer_operation_with_snapshot(
            OrganizerOperationInsert {
                rule_id: Some(rule_id),
                operation_type: OrganizerOperationType::Move,
                source_path,
                destination_path,
                original_operation_id: None,
                source_snapshot_before: None,
            },
            status,
            error,
        )
    }

    fn record_terminal_organizer_operation_with_snapshot(
        &self,
        operation: OrganizerOperationInsert<'_>,
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
                (operation_id, rule_id, source_path, destination_path, operation_type, status,
                 created_at, started_at, finished_at, error, original_operation_id,
                 effective_source_path, effective_destination_path, source_snapshot_before,
                 source_path_bytes, destination_path_bytes, effective_source_path_bytes,
                 effective_destination_path_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?7, ?8, ?9, ?3, ?4, ?10, ?11, ?12, ?11, ?12)",
            params![
                operation_id_text(operation_id),
                operation.rule_id,
                path_text(operation.source_path),
                path_text(operation.destination_path),
                operation.operation_type.as_str(),
                status.as_str(),
                finished_at,
                error,
                operation.original_operation_id.map(operation_id_text),
                operation
                    .source_snapshot_before
                    .map(|snapshot| snapshot.to_bytes().to_vec()),
                path_bytes(operation.source_path),
                path_bytes(operation.destination_path),
            ],
        )?;
        tx.commit()?;
        Ok(operation_id)
    }

    pub fn create_retry_organizer_operation(
        &self,
        original_operation_id: OrganizerOperationId,
        rule_id: i64,
        source_path: &Path,
        destination_path: &Path,
        source_snapshot_before: OrganizerFileSnapshot,
    ) -> Result<OrganizerOperationId, OrganizerOperationDbError> {
        let started_at = now_unix_millis()?;
        let mut db = self
            .writer
            .lock()
            .map_err(|_| OrganizerOperationDbError::DatabaseUnavailable)?;
        let tx = db.transaction()?;
        let original = tx
            .query_row(
                "SELECT status, operation_type, rule_id, effective_source_path_bytes,
                        source_snapshot_before
                 FROM organizer_operations WHERE operation_id = ?1",
                params![operation_id_text(original_operation_id)],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<Vec<u8>>>(3)?,
                        row.get::<_, Option<Vec<u8>>>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            status,
            operation_type,
            original_rule_id,
            original_source_path,
            original_snapshot,
        )) = original
        else {
            return Err(OrganizerOperationDbError::NotFound(original_operation_id));
        };
        if !matches!(status.as_str(), "failed" | "skipped" | "cancelled")
            || operation_type == OrganizerOperationType::Undo.as_str()
            || original_rule_id != Some(rule_id)
            || original_source_path.as_deref() != Some(path_bytes(source_path).as_slice())
            || original_snapshot.as_deref() != Some(source_snapshot_before.to_bytes().as_slice())
        {
            return Err(OrganizerOperationDbError::RetryUnavailable(
                original_operation_id,
            ));
        }
        let retry_in_progress: bool = tx.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM organizer_operations
                 WHERE original_operation_id = ?1 AND status = 'started'
             )",
            params![operation_id_text(original_operation_id)],
            |row| row.get(0),
        )?;
        if retry_in_progress {
            return Err(OrganizerOperationDbError::RetryUnavailable(
                original_operation_id,
            ));
        }
        let operation_id = insert_started_operation(
            &tx,
            started_at,
            OrganizerOperationInsert {
                rule_id: Some(rule_id),
                operation_type: OrganizerOperationType::Retry,
                source_path,
                destination_path,
                original_operation_id: Some(original_operation_id),
                source_snapshot_before: Some(source_snapshot_before),
            },
        )?;
        tx.commit()?;
        Ok(operation_id)
    }

    pub fn create_undo_organizer_operation(
        &self,
        original_operation_id: OrganizerOperationId,
        source_path: &Path,
        destination_path: &Path,
        source_snapshot_before: OrganizerFileSnapshot,
    ) -> Result<OrganizerOperationId, OrganizerOperationDbError> {
        let started_at = now_unix_millis()?;
        let mut db = self
            .writer
            .lock()
            .map_err(|_| OrganizerOperationDbError::DatabaseUnavailable)?;
        let tx = db.transaction()?;
        let original = tx
            .query_row(
                "SELECT status, operation_type, rule_id, effective_source_path, effective_destination_path,
                        effective_source_path_bytes, effective_destination_path_bytes,
                        destination_snapshot_after, undone_at
                 FROM organizer_operations WHERE operation_id = ?1",
                params![operation_id_text(original_operation_id)],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<Vec<u8>>>(5)?,
                        row.get::<_, Option<Vec<u8>>>(6)?,
                        row.get::<_, Option<Vec<u8>>>(7)?,
                        row.get::<_, Option<i64>>(8)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            status,
            operation_type,
            rule_id,
            effective_source_path,
            effective_destination_path,
            effective_source_path_bytes,
            effective_destination_path_bytes,
            destination_snapshot_after,
            undone_at,
        )) = original
        else {
            return Err(OrganizerOperationDbError::NotFound(original_operation_id));
        };
        let Some(destination_snapshot_after) =
            destination_snapshot_after.and_then(|bytes| OrganizerFileSnapshot::from_bytes(&bytes))
        else {
            return Err(OrganizerOperationDbError::UndoUnavailable(
                original_operation_id,
            ));
        };
        if status != OrganizerOperationStatus::Completed.as_str()
            || operation_type == OrganizerOperationType::Undo.as_str()
            || effective_source_path.is_none()
            || effective_destination_path.is_none()
            || undone_at.is_some()
            || effective_source_path_bytes.as_deref()
                != Some(path_bytes(destination_path).as_slice())
            || effective_destination_path_bytes.as_deref()
                != Some(path_bytes(source_path).as_slice())
            || source_snapshot_before != destination_snapshot_after
        {
            return Err(OrganizerOperationDbError::UndoUnavailable(
                original_operation_id,
            ));
        }
        let undo_exists: bool = tx.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM organizer_operations
                 WHERE original_operation_id = ?1
                   AND operation_type = 'undo'
                   AND status IN ('started', 'completed')
             )",
            params![operation_id_text(original_operation_id)],
            |row| row.get(0),
        )?;
        if undo_exists {
            return Err(OrganizerOperationDbError::UndoUnavailable(
                original_operation_id,
            ));
        }
        let operation_id = insert_started_operation(
            &tx,
            started_at,
            OrganizerOperationInsert {
                rule_id,
                operation_type: OrganizerOperationType::Undo,
                source_path,
                destination_path,
                original_operation_id: Some(original_operation_id),
                source_snapshot_before: Some(source_snapshot_before),
            },
        )?;
        tx.commit()?;
        Ok(operation_id)
    }

    pub(crate) fn record_organizer_operation_completion(
        &self,
        operation_id: OrganizerOperationId,
        source_path: &Path,
        destination_path: &Path,
        destination_snapshot: OrganizerFileSnapshot,
    ) -> Result<(), OrganizerOperationDbError> {
        let db = self
            .writer
            .lock()
            .map_err(|_| OrganizerOperationDbError::DatabaseUnavailable)?;
        db.execute(
            "INSERT OR REPLACE INTO organizer_operation_completions
                 (operation_id, source_path, destination_path, source_path_bytes,
                  destination_path_bytes, destination_snapshot)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                operation_id_text(operation_id),
                path_text(source_path),
                path_text(destination_path),
                path_bytes(source_path),
                path_bytes(destination_path),
                destination_snapshot.to_bytes().to_vec(),
            ],
        )?;
        Ok(())
    }

    pub(crate) fn reconcile_organizer_operation_completion(
        &self,
        operation_id: OrganizerOperationId,
    ) -> Result<bool, OrganizerOperationDbError> {
        let completion = {
            let db = self
                .reader
                .lock()
                .map_err(|_| OrganizerOperationDbError::DatabaseUnavailable)?;
            db.query_row(
                "SELECT source_path, source_path_bytes, destination_path,
                        destination_path_bytes, destination_snapshot
                 FROM organizer_operation_completions WHERE operation_id = ?1",
                params![operation_id_text(operation_id)],
                |row| {
                    let source_path = path_from_storage(row.get(0)?, row.get(1)?, 1)?;
                    let destination_path = path_from_storage(row.get(2)?, row.get(3)?, 3)?;
                    let snapshot_bytes = row.get::<_, Vec<u8>>(4)?;
                    let destination_snapshot = OrganizerFileSnapshot::from_bytes(&snapshot_bytes)
                        .ok_or_else(|| {
                        malformed_blob(4, "invalid organizer completion snapshot")
                    })?;
                    Ok((source_path, destination_path, destination_snapshot))
                },
            )
            .optional()?
        };
        let Some((source_path, destination_path, destination_snapshot)) = completion else {
            return Ok(false);
        };
        let source_snapshot = self
            .get_organizer_operation(operation_id)?
            .and_then(|operation| operation.source_snapshot_before);
        if organizer_file_snapshot(&destination_path).ok() != Some(destination_snapshot) {
            return Ok(false);
        }
        if source_snapshot.is_some()
            && organizer_file_snapshot(&source_path).ok() == source_snapshot
        {
            remove_organizer_file_if_matches(&destination_path, destination_snapshot)?;
            return Ok(false);
        }
        if source_snapshot.is_none() {
            return Ok(false);
        }

        match self.finish_organizer_operation_with_metadata(
            operation_id,
            OrganizerOperationStatus::Completed,
            None,
            Some(&source_path),
            Some(&destination_path),
            Some(destination_snapshot),
        ) {
            Ok(()) => Ok(true),
            Err(OrganizerOperationDbError::AlreadyFinalized(_)) => Ok(self
                .get_organizer_operation(operation_id)?
                .is_some_and(|operation| operation.status == OrganizerOperationStatus::Completed)),
            Err(error) => Err(error),
        }
    }

    pub fn reconcile_started_organizer_operations(
        &self,
    ) -> Result<usize, OrganizerOperationDbError> {
        let interrupted = {
            let db = self
                .reader
                .lock()
                .map_err(|_| OrganizerOperationDbError::DatabaseUnavailable)?;
            let mut statement = db.prepare(
                "SELECT operation_id, owner_id FROM organizer_operations WHERE status = 'started'",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    parse_operation_id(row.get(0)?, 0)?,
                    row.get::<_, String>(1)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let mut reconciled = 0usize;
        for (operation_id, owner_id) in interrupted {
            if process_owner_is_active(&owner_id) {
                continue;
            }
            match self.reconcile_organizer_operation_completion(operation_id) {
                Ok(true) => {
                    reconciled = reconciled.saturating_add(1);
                    continue;
                }
                Ok(false) => {}
                Err(OrganizerOperationDbError::Filesystem(error)) => {
                    log::warn!(
                        "[ORGANIZER] Deferred interrupted operation {} cleanup: {}",
                        operation_id,
                        error
                    );
                    continue;
                }
                Err(error) => return Err(error),
            }
            match self.finish_organizer_operation(
                operation_id,
                OrganizerOperationStatus::Failed,
                Some("operation owner exited before completion was persisted"),
            ) {
                Ok(()) => reconciled = reconciled.saturating_add(1),
                Err(OrganizerOperationDbError::AlreadyFinalized(_)) => {}
                Err(error) => return Err(error),
            }
        }
        let db = self
            .writer
            .lock()
            .map_err(|_| OrganizerOperationDbError::DatabaseUnavailable)?;
        db.execute(
            "DELETE FROM organizer_operation_completions
             WHERE NOT EXISTS (
                 SELECT 1 FROM organizer_operations o
                 WHERE o.operation_id = organizer_operation_completions.operation_id
                   AND o.status = 'started'
             )",
            [],
        )?;
        Ok(reconciled)
    }
}
