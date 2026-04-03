use rusqlite::params;

use super::IndexDb;
use crate::file_index::VolumeIndex;

impl IndexDb {
    /// Save the complete volume index to the database.
    ///
    /// This is expensive for large volumes (DELETE ALL + INSERT ALL + FTS5 rebuild).
    /// Use only for initial scan or service shutdown.  For periodic persist, prefer
    /// `save_volume_state` + `sync_fts_incremental`.
    pub fn save_volume(&self, index: &VolumeIndex) -> Result<(), String> {
        let conn = self.conn.lock();

        let drive = index.drive_letter.to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        conn.execute(
            "INSERT OR REPLACE INTO volume_state
             (drive_letter, journal_id, last_usn, files_indexed, last_full_scan_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                drive,
                index.journal_id as i64,
                index.last_usn,
                index.records.len() as i64,
                now
            ],
        )
        .map_err(|e| format!("Save volume_state error: {}", e))?;

        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("Transaction begin error: {}", e))?;

        tx.execute(
            "DELETE FROM file_records WHERE drive_letter = ?1",
            params![drive],
        )
        .map_err(|e| format!("Delete old records error: {}", e))?;

        {
            let mut insert_stmt = tx
                .prepare(
                    "INSERT INTO file_records (frn, drive_letter, name, parent_frn, is_dir)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )
                .map_err(|e| format!("Prepare insert error: {}", e))?;

            for (&frn, record) in &index.records {
                let name = index.names.get(record.name_ref());
                insert_stmt
                    .execute(params![
                        frn as i64,
                        drive,
                        name,
                        record.parent_ref as i64,
                        record.is_dir
                    ])
                    .map_err(|e| format!("Insert record error: {}", e))?;
            }
        }

        tx.commit()
            .map_err(|e| format!("Transaction commit error: {}", e))?;

        let fts_start = std::time::Instant::now();
        conn.execute(
            "INSERT INTO search_fts(search_fts) VALUES('rebuild')",
            [],
        )
        .map_err(|e| format!("FTS5 rebuild error after save: {}", e))?;

        eprintln!(
            "[DB] Saved {} records for volume {}:\\ (FTS5 rebuilt in {:.2}s)",
            index.records.len(),
            index.drive_letter,
            fts_start.elapsed().as_secs_f64()
        );
        Ok(())
    }

    pub fn save_volume_state_snapshot(
        &self,
        drive_letter: char,
        journal_id: u64,
        last_usn: i64,
        files_indexed: usize,
    ) -> Result<(), String> {
        let conn = self.conn.lock();

        let drive = drive_letter.to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        conn.execute(
            "INSERT OR REPLACE INTO volume_state
             (drive_letter, journal_id, last_usn, files_indexed, last_full_scan_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![drive, journal_id as i64, last_usn, files_indexed as i64, now],
        )
        .map_err(|e| format!("Save volume_state error: {}", e))?;

        Ok(())
    }

    pub fn sync_fts_incremental_snapshot(
        &self,
        drive_letter: char,
        additions: &[(u64, String, u64, bool)],
        removals: &std::collections::HashSet<u64>,
    ) -> Result<(), String> {
        if additions.is_empty() && removals.is_empty() {
            return Ok(());
        }

        let conn = self.conn.lock();
        let drive = drive_letter.to_string();

        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("Transaction begin error: {}", e))?;

        let mut removed_count = 0usize;
        let mut added_count = 0usize;
        let mut updated_count = 0usize;

        // --- Process removals ---
        for &frn in removals {
            let existing: Option<(i64, String)> = tx
                .query_row(
                    "SELECT rowid, name FROM file_records WHERE drive_letter = ?1 AND frn = ?2",
                    params![drive, frn as i64],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok();

            if let Some((rowid, old_name)) = existing {
                let _ = tx.execute(
                    "INSERT INTO search_fts(search_fts, rowid, name) VALUES('delete', ?1, ?2)",
                    params![rowid, old_name],
                );
                tx.execute(
                    "DELETE FROM file_records WHERE drive_letter = ?1 AND frn = ?2",
                    params![drive, frn as i64],
                )
                .map_err(|e| format!("Delete record error: {}", e))?;
                removed_count += 1;
            }
        }

        // --- Process additions (new + updated records) ---
        for (frn, name, parent_ref, is_dir) in additions {
            let frn = *frn;
            let parent_ref = *parent_ref;
            let is_dir = *is_dir;

            let existing: Option<(i64, String)> = tx
                .query_row(
                    "SELECT rowid, name FROM file_records WHERE drive_letter = ?1 AND frn = ?2",
                    params![drive, frn as i64],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok();

            if let Some((rowid, old_name)) = existing {
                tx.execute(
                    "UPDATE file_records SET name = ?1, parent_frn = ?2, is_dir = ?3
                     WHERE drive_letter = ?4 AND frn = ?5",
                    params![name, parent_ref as i64, is_dir, drive, frn as i64],
                )
                .map_err(|e| format!("Update record error: {}", e))?;

                let _ = tx.execute(
                    "INSERT INTO search_fts(search_fts, rowid, name) VALUES('delete', ?1, ?2)",
                    params![rowid, old_name],
                );
                let _ = tx.execute(
                    "INSERT INTO search_fts(rowid, name) VALUES(?1, ?2)",
                    params![rowid, name],
                );
                updated_count += 1;
            } else {
                tx.execute(
                    "INSERT INTO file_records (frn, drive_letter, name, parent_frn, is_dir)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![frn as i64, drive, name, parent_ref as i64, is_dir],
                )
                .map_err(|e| format!("Insert record error: {}", e))?;

                let new_rowid = tx.last_insert_rowid();
                let _ = tx.execute(
                    "INSERT INTO search_fts(rowid, name) VALUES(?1, ?2)",
                    params![new_rowid, name],
                );
                added_count += 1;
            }
        }

        tx.commit()
            .map_err(|e| format!("Transaction commit error: {}", e))?;

        if removed_count > 0 || added_count > 0 || updated_count > 0 {
            eprintln!(
                "[DB] {}:\\ Incremental sync: +{} ~{} -{} records",
                drive_letter, added_count, updated_count, removed_count
            );
        }

        Ok(())
    }
}
