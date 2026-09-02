use super::organizer_conflicts::{
    conflict_id_text, now_unix_millis, optional_snapshot_from_storage, parse_conflict_id,
    snapshot_from_storage, OrganizerConflictDbError,
};
use super::organizer_operations::{path_bytes, path_from_storage, path_text};
use super::{process_owner_id, process_owner_is_active, AppStateDb};
use crate::domain::organizer_conflict::OrganizerConflictId;
use crate::infrastructure::windows::shell_operations::organizer_file_snapshot;
use rusqlite::{params, OptionalExtension};
use std::path::Path;

impl AppStateDb {
    pub fn claim_organizer_conflict_resolution(
        &self,
        conflict_id: OrganizerConflictId,
        rename_source: bool,
        target_path: &Path,
    ) -> Result<(), OrganizerConflictDbError> {
        let started_at = now_unix_millis()?;
        let mut db = self
            .writer
            .lock()
            .map_err(|_| OrganizerConflictDbError::DatabaseUnavailable)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (status, source_snapshot, destination_snapshot) = tx
            .query_row(
                "SELECT status, source_snapshot, destination_snapshot
                 FROM organizer_conflicts WHERE conflict_id = ?1",
                params![conflict_id_text(conflict_id)],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        snapshot_from_storage(row.get(1)?, 1)?,
                        optional_snapshot_from_storage(row.get(2)?, 2)?,
                    ))
                },
            )
            .optional()?
            .ok_or(OrganizerConflictDbError::NotFound(conflict_id))?;
        if status != "pending" {
            return Err(OrganizerConflictDbError::AlreadyFinalized(conflict_id));
        }
        let expected_snapshot = if rename_source {
            source_snapshot
        } else {
            destination_snapshot.ok_or(OrganizerConflictDbError::AlreadyFinalized(conflict_id))?
        };
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO organizer_conflict_resolutions
                (conflict_id, rename_source, target_path, target_path_bytes, expected_snapshot,
                 owner_id, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                conflict_id_text(conflict_id),
                rename_source,
                path_text(target_path),
                path_bytes(target_path),
                expected_snapshot.to_bytes().to_vec(),
                process_owner_id(),
                started_at,
            ],
        )?;
        if inserted != 1 {
            return Err(OrganizerConflictDbError::ResolutionInProgress(conflict_id));
        }
        tx.commit()?;
        Ok(())
    }

    pub fn release_organizer_conflict_resolution(
        &self,
        conflict_id: OrganizerConflictId,
    ) -> Result<(), OrganizerConflictDbError> {
        let db = self
            .writer
            .lock()
            .map_err(|_| OrganizerConflictDbError::DatabaseUnavailable)?;
        db.execute(
            "DELETE FROM organizer_conflict_resolutions
             WHERE conflict_id = ?1 AND owner_id = ?2",
            params![conflict_id_text(conflict_id), process_owner_id()],
        )?;
        Ok(())
    }

    pub fn reconcile_organizer_conflict_resolutions(&self) -> Result<(), OrganizerConflictDbError> {
        self.reconcile_organizer_conflict_resolution_rows(None)
    }

    pub fn reconcile_owned_organizer_conflict_resolution(
        &self,
        conflict_id: OrganizerConflictId,
    ) -> Result<(), OrganizerConflictDbError> {
        self.reconcile_organizer_conflict_resolution_rows(Some(conflict_id))
    }

    fn reconcile_organizer_conflict_resolution_rows(
        &self,
        owned_conflict_id: Option<OrganizerConflictId>,
    ) -> Result<(), OrganizerConflictDbError> {
        let checked_at = now_unix_millis()?;
        let resolutions = {
            let db = self
                .writer
                .lock()
                .map_err(|_| OrganizerConflictDbError::DatabaseUnavailable)?;
            let mut statement = db.prepare(
                "SELECT r.conflict_id, r.rename_source, r.target_path, r.target_path_bytes,
                        r.expected_snapshot, r.owner_id,
                        c.source_path, c.source_path_bytes, c.destination_path,
                        c.destination_path_bytes, c.source_snapshot, c.destination_snapshot
                 FROM organizer_conflict_resolutions r
                 JOIN organizer_conflicts c ON c.conflict_id = r.conflict_id",
            )?;
            let rows = statement.query_map([], |row| {
                let conflict_id = parse_conflict_id(row.get(0)?, 0)?;
                let rename_source = row.get::<_, bool>(1)?;
                let target = path_from_storage(row.get(2)?, row.get(3)?, 3)?;
                let expected_snapshot = optional_snapshot_from_storage(row.get(4)?, 4)?;
                let owner_id = row.get::<_, String>(5)?;
                let source = path_from_storage(row.get(6)?, row.get(7)?, 7)?;
                let destination = path_from_storage(row.get(8)?, row.get(9)?, 9)?;
                let source_snapshot = snapshot_from_storage(row.get(10)?, 10)?;
                let destination_snapshot = optional_snapshot_from_storage(row.get(11)?, 11)?;
                Ok((
                    conflict_id,
                    rename_source,
                    target,
                    expected_snapshot,
                    owner_id,
                    source,
                    destination,
                    source_snapshot,
                    destination_snapshot,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        for (
            conflict_id,
            rename_source,
            target,
            expected_snapshot,
            owner_id,
            source,
            destination,
            source_snapshot,
            destination_snapshot,
        ) in resolutions
        {
            if let Some(owned_conflict_id) = owned_conflict_id {
                if conflict_id != owned_conflict_id || owner_id != process_owner_id() {
                    continue;
                }
            } else if process_owner_is_active(&owner_id) {
                continue;
            }
            let original = if rename_source { &source } else { &destination };
            let expected_original = if rename_source {
                Some(source_snapshot)
            } else {
                destination_snapshot
            };
            let original_exists = original.try_exists().unwrap_or(true);
            let original_snapshot = organizer_file_snapshot(original).ok();
            let target_exists = target.try_exists().unwrap_or(true);
            let target_snapshot = organizer_file_snapshot(&target).ok();
            let status = if !original_exists
                && expected_snapshot.is_some()
                && target_snapshot == expected_snapshot
            {
                "resolved"
            } else if original_snapshot == expected_original && !target_exists {
                let db = self
                    .writer
                    .lock()
                    .map_err(|_| OrganizerConflictDbError::DatabaseUnavailable)?;
                db.execute(
                    "DELETE FROM organizer_conflict_resolutions
                     WHERE conflict_id = ?1 AND owner_id = ?2",
                    params![conflict_id_text(conflict_id), owner_id],
                )?;
                continue;
            } else {
                "obsolete"
            };
            let mut db = self
                .writer
                .lock()
                .map_err(|_| OrganizerConflictDbError::DatabaseUnavailable)?;
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tx.execute(
                "UPDATE organizer_conflicts
                 SET status = ?1, last_checked_at = ?2
                 WHERE conflict_id = ?3 AND status = 'pending' AND EXISTS (
                     SELECT 1 FROM organizer_conflict_resolutions
                     WHERE conflict_id = ?3 AND owner_id = ?4
                 )",
                params![status, checked_at, conflict_id_text(conflict_id), owner_id],
            )?;
            tx.execute(
                "DELETE FROM organizer_conflict_resolutions
                 WHERE conflict_id = ?1 AND owner_id = ?2",
                params![conflict_id_text(conflict_id), owner_id],
            )?;
            tx.commit()?;
        }
        Ok(())
    }
}
