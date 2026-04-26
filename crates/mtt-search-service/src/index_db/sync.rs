use rusqlite::params;

use super::IndexDb;
use crate::file_index::VolumeIndex;

impl IndexDb {
    /// Save the complete volume index to the database (records only, no FTS).
    ///
    /// Replaces all `file_records` and `hardlink_parents` for this volume.
    /// FTS5 is **not** touched here — call `rebuild_fts_full()` separately
    /// (typically from a background thread) so the user can search via the
    /// in-memory linear scan while the FTS index is being rebuilt.
    ///
    /// The work is split into batched commits so the WAL stays bounded.
    /// `on_progress(current, total)` reports insert progress.
    pub fn save_volume<F>(&self, index: &VolumeIndex, mut on_progress: F) -> Result<(), String>
    where
        F: FnMut(u64, u64),
    {
        let conn = self.conn.lock();

        let drive = index.drive_letter.to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        conn.execute(
            "INSERT OR REPLACE INTO volume_state
             (drive_letter, journal_id, last_usn, files_indexed, last_full_scan_epoch, has_hardlink_parent_data, has_reparse_point_data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                drive,
                index.journal_id as i64,
                index.last_usn,
                index.records.len() as i64,
                now,
                index.hardlink_data_complete,
                index.reparse_data_complete,
            ],
        )
        .map_err(|e| format!("Save volume_state error: {}", e))?;

        let total = index.records.len() as u64;
        on_progress(0, total);

        // ── Phase 1: delete old records ─────────────────────────────────
        {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("Transaction begin (cleanup) error: {}", e))?;

            tx.execute(
                "DELETE FROM file_records WHERE drive_letter = ?1",
                params![drive],
            )
            .map_err(|e| format!("Delete old records error: {}", e))?;

            tx.execute(
                "DELETE FROM hardlink_parents WHERE drive_letter = ?1",
                params![drive],
            )
            .map_err(|e| format!("Delete old hardlink parents error: {}", e))?;

            tx.commit()
                .map_err(|e| format!("Cleanup commit error: {}", e))?;
        }

        // ── Phase 2: insert new records, batched commits ────────────────
        const COMMIT_BATCH: u64 = 32_768;
        let mut inserted = 0u64;
        let mut last_reported = 0u64;
        let mut last_report_at = std::time::Instant::now();

        let mut records_iter = index.records.iter().peekable();

        while records_iter.peek().is_some() {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("Transaction begin (insert batch) error: {}", e))?;
            {
                let mut insert_stmt = tx
                    .prepare(
                        "INSERT INTO file_records (frn, drive_letter, name, parent_frn, is_dir, is_reparse)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    )
                    .map_err(|e| format!("Prepare insert error: {}", e))?;

                let mut batch_count = 0u64;
                while batch_count < COMMIT_BATCH {
                    let (&frn, record) = match records_iter.next() {
                        Some(entry) => entry,
                        None => break,
                    };

                    let name = index.names.get(record.name_ref());
                    insert_stmt
                        .execute(params![
                            frn as i64,
                            drive,
                            name,
                            record.parent_ref as i64,
                            record.is_dir,
                            index.reparse_points.contains(&frn)
                        ])
                        .map_err(|e| format!("Insert record error: {}", e))?;

                    inserted += 1;
                    batch_count += 1;

                    if inserted.saturating_sub(last_reported) >= 8_192
                        || last_report_at.elapsed() >= std::time::Duration::from_millis(200)
                    {
                        on_progress(inserted, total);
                        last_reported = inserted;
                        last_report_at = std::time::Instant::now();
                    }
                }
            } // drop statements before commit
            tx.commit()
                .map_err(|e| format!("Insert batch commit error: {}", e))?;
        }

        if inserted != last_reported {
            on_progress(inserted, total);
        }

        // ── Hardlink parents (single transaction) ───────────────────────
        if !index.hardlink_parents.is_empty() {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("Transaction begin (hardlinks) error: {}", e))?;
            {
                let mut hardlink_stmt = tx
                    .prepare(
                        "INSERT INTO hardlink_parents (frn, drive_letter, parent_frn)
                         VALUES (?1, ?2, ?3)",
                    )
                    .map_err(|e| format!("Prepare hardlink insert error: {}", e))?;

                for (&frn, parents) in &index.hardlink_parents {
                    let primary_parent = index.records.get(&frn).map(|record| record.parent_ref);
                    let mut unique_parents = parents.clone();
                    unique_parents.sort_unstable();
                    unique_parents.dedup();
                    for parent_ref in unique_parents {
                        if Some(parent_ref) == primary_parent
                            || parent_ref == 0
                            || parent_ref == frn
                        {
                            continue;
                        }
                        hardlink_stmt
                            .execute(params![frn as i64, drive, parent_ref as i64])
                            .map_err(|e| format!("Insert hardlink parent error: {}", e))?;
                    }
                }
            }
            tx.commit()
                .map_err(|e| format!("Hardlink commit error: {}", e))?;
        }

        eprintln!(
            "[DB] Saved {} records for volume {}:\\ (records only, FTS deferred)",
            index.records.len(),
            index.drive_letter,
        );
        Ok(())
    }

    /// Rebuild the FTS5 index from the current `file_records` table.
    ///
    /// This is expensive (~25-60 s for ~1.7 M records with the trigram
    /// tokenizer) so it should be called from a **background thread** after
    /// the volume has already been marked `Ready`.
    pub fn rebuild_fts_full(&self) -> Result<std::time::Duration, String> {
        let conn = self.conn.lock();
        let start = std::time::Instant::now();
        conn.execute("INSERT INTO search_fts(search_fts) VALUES('rebuild')", [])
            .map_err(|e| format!("FTS5 rebuild error: {}", e))?;
        let elapsed = start.elapsed();
        eprintln!(
            "[DB] FTS5 background rebuild completed in {:.2}s",
            elapsed.as_secs_f64()
        );
        Ok(elapsed)
    }

    pub fn save_volume_state_snapshot(
        &self,
        drive_letter: char,
        journal_id: u64,
        last_usn: i64,
        files_indexed: usize,
        has_hardlink_parent_data: bool,
        has_reparse_point_data: bool,
    ) -> Result<(), String> {
        let conn = self.conn.lock();

        let drive = drive_letter.to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        conn.execute(
            "INSERT OR REPLACE INTO volume_state
             (drive_letter, journal_id, last_usn, files_indexed, last_full_scan_epoch, has_hardlink_parent_data, has_reparse_point_data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                drive,
                journal_id as i64,
                last_usn,
                files_indexed as i64,
                now,
                has_hardlink_parent_data,
                has_reparse_point_data,
            ],
        )
        .map_err(|e| format!("Save volume_state error: {}", e))?;

        Ok(())
    }

    pub fn sync_fts_incremental_snapshot(
        &self,
        drive_letter: char,
        additions: &[(u64, String, u64, bool, bool, Vec<u64>)],
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
                tx.execute(
                    "DELETE FROM hardlink_parents WHERE drive_letter = ?1 AND frn = ?2",
                    params![drive, frn as i64],
                )
                .map_err(|e| format!("Delete hardlink parent error: {}", e))?;
                removed_count += 1;
            }
        }

        // --- Process additions (new + updated records) ---
        for (frn, name, parent_ref, is_dir, is_reparse, extra_parents) in additions {
            let frn = *frn;
            let parent_ref = *parent_ref;
            let is_dir = *is_dir;
            let is_reparse = *is_reparse;

            let existing: Option<(i64, String)> = tx
                .query_row(
                    "SELECT rowid, name FROM file_records WHERE drive_letter = ?1 AND frn = ?2",
                    params![drive, frn as i64],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok();

            if let Some((rowid, old_name)) = existing {
                tx.execute(
                    "UPDATE file_records SET name = ?1, parent_frn = ?2, is_dir = ?3, is_reparse = ?4
                     WHERE drive_letter = ?5 AND frn = ?6",
                    params![name, parent_ref as i64, is_dir, is_reparse, drive, frn as i64],
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
                    "INSERT INTO file_records (frn, drive_letter, name, parent_frn, is_dir, is_reparse)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![frn as i64, drive, name, parent_ref as i64, is_dir, is_reparse],
                )
                .map_err(|e| format!("Insert record error: {}", e))?;

                let new_rowid = tx.last_insert_rowid();
                let _ = tx.execute(
                    "INSERT INTO search_fts(rowid, name) VALUES(?1, ?2)",
                    params![new_rowid, name],
                );
                added_count += 1;
            }

            tx.execute(
                "DELETE FROM hardlink_parents WHERE drive_letter = ?1 AND frn = ?2",
                params![drive, frn as i64],
            )
            .map_err(|e| format!("Delete stale hardlink parents error: {}", e))?;

            let mut unique_parents = extra_parents.clone();
            unique_parents.sort_unstable();
            unique_parents.dedup();
            for extra_parent in unique_parents {
                if extra_parent == frn || extra_parent == 0 || extra_parent == parent_ref {
                    continue;
                }
                tx.execute(
                    "INSERT INTO hardlink_parents (frn, drive_letter, parent_frn)
                     VALUES (?1, ?2, ?3)",
                    params![frn as i64, drive, extra_parent as i64],
                )
                .map_err(|e| format!("Insert incremental hardlink parent error: {}", e))?;
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
