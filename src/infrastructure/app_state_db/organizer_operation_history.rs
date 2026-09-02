use super::{
    malformed_blob, now_unix_millis, operation_id_text, path_from_storage, record_from_row,
    OrganizerOperationDbError, OrganizerOperationRecord, DEFAULT_ORGANIZER_HISTORY_RETENTION_DAYS,
    MAX_ORGANIZER_HISTORY_RETENTION_DAYS, MIN_ORGANIZER_HISTORY_RETENTION_DAYS,
    ORGANIZER_HISTORY_RETENTION_PREFERENCE,
};
use crate::infrastructure::app_state_db::{AppStateDb, AppStateWriteError};
use rusqlite::{params, OptionalExtension};
use std::sync::atomic::{AtomicU64, Ordering};

static ORGANIZER_HISTORY_REVISION: AtomicU64 = AtomicU64::new(0);

impl AppStateDb {
    pub fn get_organizer_operation(
        &self,
        operation_id: crate::domain::organizer_operation::OrganizerOperationId,
    ) -> Result<Option<OrganizerOperationRecord>, OrganizerOperationDbError> {
        let db = self
            .reader
            .lock()
            .map_err(|_| OrganizerOperationDbError::DatabaseUnavailable)?;
        db.query_row(
            "SELECT operation_id, rule_id, source_path, destination_path, operation_type,
                    status, created_at, started_at, finished_at, error, conflict_id,
                    original_operation_id, effective_source_path, effective_destination_path,
                    source_snapshot_before, destination_snapshot_after, source_path_bytes,
                    destination_path_bytes, effective_source_path_bytes,
                    effective_destination_path_bytes, undone_at
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
            "SELECT operation_id, rule_id, source_path, destination_path, operation_type,
                    status, created_at, started_at, finished_at, error, conflict_id,
                    original_operation_id, effective_source_path, effective_destination_path,
                    source_snapshot_before, destination_snapshot_after, source_path_bytes,
                    destination_path_bytes, effective_source_path_bytes,
                    effective_destination_path_bytes, undone_at
             FROM organizer_operations
             ORDER BY created_at DESC, CAST(operation_id AS INTEGER) DESC
             LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit], record_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(OrganizerOperationDbError::from)
    }

    pub(crate) fn list_organizer_undo_exemptions(
        &self,
    ) -> Result<
        Vec<(
            std::path::PathBuf,
            crate::infrastructure::windows::shell_operations::OrganizerFileSnapshot,
        )>,
        OrganizerOperationDbError,
    > {
        let db = self
            .reader
            .lock()
            .map_err(|_| OrganizerOperationDbError::DatabaseUnavailable)?;
        let mut statement =
            db.prepare("SELECT path, path_bytes, snapshot FROM organizer_undo_exemptions")?;
        let rows = statement.query_map([], |row| {
            let path = path_from_storage(row.get(0)?, row.get(1)?, 1)?;
            let bytes = row.get::<_, Vec<u8>>(2)?;
            let snapshot = crate::infrastructure::windows::shell_operations::OrganizerFileSnapshot::from_bytes(&bytes)
                .ok_or_else(|| malformed_blob(2, "invalid organizer operation snapshot"))?;
            Ok((path, snapshot))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(OrganizerOperationDbError::from)
    }

    pub(crate) fn remove_organizer_undo_exemption(
        &self,
        path: &std::path::Path,
    ) -> Result<(), OrganizerOperationDbError> {
        let db = self
            .writer
            .lock()
            .map_err(|_| OrganizerOperationDbError::DatabaseUnavailable)?;
        db.execute(
            "DELETE FROM organizer_undo_exemptions WHERE path_bytes = ?1",
            params![super::path_bytes(path)],
        )?;
        Ok(())
    }

    pub(crate) fn save_organizer_undo_exemption(
        &self,
        path: &std::path::Path,
        snapshot: crate::infrastructure::windows::shell_operations::OrganizerFileSnapshot,
    ) -> Result<(), OrganizerOperationDbError> {
        let db = self
            .writer
            .lock()
            .map_err(|_| OrganizerOperationDbError::DatabaseUnavailable)?;
        db.execute(
            "INSERT OR REPLACE INTO organizer_undo_exemptions (path_bytes, path, snapshot)
             VALUES (?1, ?2, ?3)",
            params![
                super::path_bytes(path),
                super::path_text(path),
                snapshot.to_bytes().to_vec(),
            ],
        )?;
        Ok(())
    }

    pub fn organizer_history_retention_days(&self) -> i64 {
        self.try_organizer_history_retention_days()
            .unwrap_or(DEFAULT_ORGANIZER_HISTORY_RETENTION_DAYS)
    }

    pub(crate) fn try_organizer_history_retention_days(
        &self,
    ) -> Result<i64, OrganizerOperationDbError> {
        let db = self
            .reader
            .lock()
            .map_err(|_| OrganizerOperationDbError::DatabaseUnavailable)?;
        let value = db
            .query_row(
                "SELECT value FROM user_preferences WHERE key = ?1",
                params![ORGANIZER_HISTORY_RETENTION_PREFERENCE],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(value) = value else {
            return Ok(DEFAULT_ORGANIZER_HISTORY_RETENTION_DAYS);
        };
        let retention_days = value
            .parse::<i64>()
            .map_err(|_| OrganizerOperationDbError::InvalidRetention)?;
        if !(MIN_ORGANIZER_HISTORY_RETENTION_DAYS..=MAX_ORGANIZER_HISTORY_RETENTION_DAYS)
            .contains(&retention_days)
        {
            return Err(OrganizerOperationDbError::InvalidRetention);
        }
        Ok(retention_days)
    }

    pub(crate) fn organizer_history_revision(&self) -> u64 {
        ORGANIZER_HISTORY_REVISION.load(Ordering::Acquire)
    }

    pub fn set_organizer_history_retention_days(
        &self,
        retention_days: i64,
    ) -> Result<(), AppStateWriteError> {
        if !(MIN_ORGANIZER_HISTORY_RETENTION_DAYS..=MAX_ORGANIZER_HISTORY_RETENTION_DAYS)
            .contains(&retention_days)
        {
            return Err(AppStateWriteError::Database(
                rusqlite::Error::InvalidParameterName(
                    "organizer history retention must be between 1 and 3650 days".to_string(),
                ),
            ));
        }
        self.set_preference(
            ORGANIZER_HISTORY_RETENTION_PREFERENCE,
            &retention_days.to_string(),
        )
    }

    pub fn retain_organizer_history(
        &self,
        retention_days: i64,
        batch_limit: usize,
    ) -> Result<usize, OrganizerOperationDbError> {
        if !(MIN_ORGANIZER_HISTORY_RETENTION_DAYS..=MAX_ORGANIZER_HISTORY_RETENTION_DAYS)
            .contains(&retention_days)
        {
            return Err(OrganizerOperationDbError::InvalidRetention);
        }
        if batch_limit == 0 {
            return Ok(0);
        }
        let now = now_unix_millis()?;
        let retention_millis = retention_days.saturating_mul(24 * 60 * 60 * 1000);
        let cutoff = now.saturating_sub(retention_millis);
        let limit = i64::try_from(batch_limit).unwrap_or(i64::MAX);
        let mut db = self
            .writer
            .lock()
            .map_err(|_| OrganizerOperationDbError::DatabaseUnavailable)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut removed = 0usize;

        let old_conflicts = {
            let mut statement = tx.prepare(
                "SELECT c.conflict_id
                 FROM organizer_conflicts c
                 JOIN organizer_operations o ON o.operation_id = c.operation_id
                 WHERE c.status <> 'pending'
                   AND c.last_checked_at < ?1
                   AND o.status <> 'started'
                   AND COALESCE(o.finished_at, o.started_at) < ?1
                 ORDER BY c.last_checked_at ASC, CAST(c.conflict_id AS INTEGER) ASC
                 LIMIT ?2",
            )?;
            let rows =
                statement.query_map(params![cutoff, limit], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for conflict_id in old_conflicts {
            tx.execute(
                "UPDATE organizer_operations SET conflict_id = NULL WHERE conflict_id = ?1",
                params![conflict_id],
            )?;
            tx.execute(
                "DELETE FROM organizer_conflict_resolutions WHERE conflict_id = ?1",
                params![conflict_id],
            )?;
            removed = removed.saturating_add(tx.execute(
                "DELETE FROM organizer_conflicts WHERE conflict_id = ?1 AND status <> 'pending'",
                params![conflict_id],
            )?);
        }

        let old_operations = {
            let mut statement = tx.prepare(
                "SELECT o.operation_id
                 FROM organizer_operations o
                 WHERE o.status <> 'started'
                   AND COALESCE(o.finished_at, o.started_at) < ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM organizer_conflicts c
                       WHERE c.operation_id = o.operation_id AND c.status = 'pending'
                   )
                   AND NOT EXISTS (
                       SELECT 1 FROM organizer_operations child
                       WHERE child.original_operation_id = o.operation_id
                   )
                 ORDER BY COALESCE(o.finished_at, o.started_at) ASC,
                          CAST(o.operation_id AS INTEGER) ASC
                 LIMIT ?2",
            )?;
            let rows =
                statement.query_map(params![cutoff, limit], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for operation_id in old_operations {
            tx.execute(
                "DELETE FROM organizer_conflict_resolutions
                 WHERE conflict_id IN (
                     SELECT conflict_id FROM organizer_conflicts WHERE operation_id = ?1
                 )",
                params![operation_id],
            )?;
            tx.execute(
                "DELETE FROM organizer_conflicts
                 WHERE operation_id = ?1 AND status <> 'pending'",
                params![operation_id],
            )?;
            removed = removed.saturating_add(tx.execute(
                "DELETE FROM organizer_operations WHERE operation_id = ?1 AND status <> 'started'",
                params![operation_id],
            )?);
        }
        tx.commit()?;
        if removed > 0 {
            ORGANIZER_HISTORY_REVISION.fetch_add(1, Ordering::AcqRel);
        }
        Ok(removed)
    }
}
