use super::{CacheTable, ThumbnailDiskCache};
use rusqlite::{params, Connection};
use std::path::Path;

impl ThumbnailDiskCache {
    pub(super) fn path_exists_fast(path: &str) -> bool {
        crate::infrastructure::onedrive::fast_path_exists(Path::new(path))
    }

    /// Extract drive root (e.g., "X:\\") from a path string.
    fn extract_drive_root(path: &str) -> Option<String> {
        if path.len() >= 3
            && path.as_bytes()[0].is_ascii_alphabetic()
            && path.as_bytes()[1] == b':'
            && (path.as_bytes()[2] == b'\\' || path.as_bytes()[2] == b'/')
        {
            Some(format!("{}:\\", path.as_bytes()[0] as char))
        } else {
            None
        }
    }

    /// Build a set of drive roots that are currently accessible.
    /// Entries on inaccessible drives (e.g., unmounted Cryptomator vaults)
    /// are skipped during GC to prevent deleting valid cached thumbnails.
    fn accessible_drives(
        paths: impl Iterator<Item = impl AsRef<str>>,
    ) -> std::collections::HashSet<String> {
        let mut checked: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
        let mut accessible = std::collections::HashSet::new();

        for path in paths {
            if let Some(root) = Self::extract_drive_root(path.as_ref()) {
                let is_ok = *checked
                    .entry(root.clone())
                    .or_insert_with(|| Self::path_exists_fast(&root));
                if is_ok {
                    accessible.insert(root);
                }
            }
        }
        accessible
    }

    /// Check if a path's drive is accessible (using a pre-built set).
    fn is_on_accessible_drive(path: &str, accessible: &std::collections::HashSet<String>) -> bool {
        match Self::extract_drive_root(path) {
            Some(root) => accessible.contains(&root),
            None => true, // Network paths, etc. â€” always check
        }
    }

    fn execute_batch_delete(db: &Connection, table: CacheTable, items: &[String]) -> usize {
        let mut count = 0;
        const BATCH_SIZE: usize = 500;

        for chunk in items.chunks(BATCH_SIZE) {
            let placeholders = std::iter::repeat_n("?", chunk.len())
                .collect::<Vec<_>>()
                .join(",");

            let sql = format!(
                "DELETE FROM {} WHERE {} IN ({})",
                table.table_name(),
                table.key_col(),
                placeholders
            );

            match db.execute(&sql, rusqlite::params_from_iter(chunk.iter())) {
                Ok(c) => count += c,
                Err(e) => log::error!(
                    "[GC] Failed to delete batch from {}: {:?}",
                    table.table_name(),
                    e
                ),
            }
        }

        count
    }

    /// Incremental GC pass: scans only a bounded sample to keep I/O low.
    /// Intended to run periodically in background idle windows.
    pub fn garbage_collect_incremental(&self, max_candidates: usize) -> usize {
        let limit = max_candidates.max(1) as i64;

        let sampled_entries: Vec<(String, String)>;
        let sampled_folder_previews: Vec<String>;

        {
            // M-17: Use writer connection so reader is always free for concurrent thumbnail lookups.
            let db = self.writer.lock();

            // M-15: ROWID modular sampling — O(log n) via rowid index instead of O(n log n).
            // Starts at a random rowid offset and scans forward; may return < limit near the
            // end of the table, which is acceptable for best-effort incremental GC.
            sampled_entries = db
                .prepare(
                    "SELECT id, path FROM thumbnails \
                     WHERE path IS NOT NULL \
                       AND rowid >= (ABS(RANDOM()) % MAX((SELECT COALESCE(MAX(rowid),0)+1 FROM thumbnails),1)) \
                     LIMIT ?1",
                )
                .and_then(|mut stmt| {
                    stmt.query_map(params![limit], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .map(|rows| rows.flatten().collect())
                })
                .unwrap_or_default();

            sampled_folder_previews = db
                .prepare(
                    "SELECT folder_path FROM folder_previews \
                     WHERE rowid >= (ABS(RANDOM()) % MAX((SELECT COALESCE(MAX(rowid),0)+1 FROM folder_previews),1)) \
                     LIMIT ?1",
                )
                .and_then(|mut stmt| {
                    stmt.query_map(params![limit], |row| row.get::<_, String>(0))
                        .map(|rows| rows.flatten().collect())
                })
                .unwrap_or_default();
        }

        if sampled_entries.is_empty() && sampled_folder_previews.is_empty() {
            return 0;
        }

        // CRITICAL: Determine which drives are currently accessible.
        // Skip orphan-checking for files on inaccessible drives (e.g., unmounted
        // Cryptomator vaults) to prevent deleting valid cached thumbnails.
        let all_paths = sampled_entries
            .iter()
            .map(|(_, p)| p.as_str())
            .chain(sampled_folder_previews.iter().map(|p| p.as_str()));
        let accessible = Self::accessible_drives(all_paths);

        let orphan_thumbs: Vec<String> = sampled_entries
            .into_iter()
            .filter(|(_, path)| {
                Self::is_on_accessible_drive(path, &accessible) && !Self::path_exists_fast(path)
            })
            .map(|(id, _)| id)
            .collect();

        let orphan_folder_previews: Vec<String> = sampled_folder_previews
            .into_iter()
            .filter(|path| {
                Self::is_on_accessible_drive(path, &accessible) && !Self::path_exists_fast(path)
            })
            .collect();

        if orphan_thumbs.is_empty() && orphan_folder_previews.is_empty() {
            return 0;
        }

        let mut removed = 0;
        // H-5: rusqlite Transaction — auto-rollback on error/panic
        let mut db = self.writer.lock();
        if let Ok(tx) = db.transaction() {
            if !orphan_thumbs.is_empty() {
                removed += Self::execute_batch_delete(&tx, CacheTable::Thumbnails, &orphan_thumbs);
            }
            if !orphan_folder_previews.is_empty() {
                removed += Self::execute_batch_delete(
                    &tx,
                    CacheTable::FolderPreviews,
                    &orphan_folder_previews,
                );
            }
            if let Err(e) = tx.commit() {
                log::error!("[GC] Incremental: transaction commit failed: {:?}", e);
            }
        }

        if removed > 0 {
            log::debug!("[GC] Incremental pass removed {} entries", removed);
        }
        removed
    }

    /// Runs VACUUM explicitly (heavy operation, call rarely).
    pub fn run_vacuum(&self) -> bool {
        self.writer.lock().execute("VACUUM", []).is_ok()
    }

    /// Full GC: scans all cache rows. Use sparingly.
    pub fn garbage_collect(&self) -> usize {
        log::info!("[GC] Starting full garbage collection...");

        let all_entries: Vec<(String, String)>;
        let all_folder_previews: Vec<String>;

        {
            // M-17: Use writer connection so the reader stays free for thumbnail lookups.
            let db = self.writer.lock();

            all_entries = db
                .prepare("SELECT id, path FROM thumbnails WHERE path IS NOT NULL")
                .and_then(|mut stmt| {
                    stmt.query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .map(|rows| rows.flatten().collect())
                })
                .unwrap_or_default();

            all_folder_previews = db
                .prepare("SELECT folder_path FROM folder_previews")
                .and_then(|mut stmt| {
                    stmt.query_map([], |row| row.get::<_, String>(0))
                        .map(|rows| rows.flatten().collect())
                })
                .unwrap_or_default();
        }

        // CRITICAL: Skip orphan-checking for files on inaccessible drives
        // (e.g., unmounted Cryptomator vaults) to prevent mass-deleting valid cache.
        let all_paths = all_entries
            .iter()
            .map(|(_, p)| p.as_str())
            .chain(all_folder_previews.iter().map(|p| p.as_str()));
        let accessible = Self::accessible_drives(all_paths);

        let orphan_thumbs: Vec<String> = all_entries
            .into_iter()
            .filter(|(_, path)| {
                Self::is_on_accessible_drive(path, &accessible) && !Self::path_exists_fast(path)
            })
            .map(|(id, _)| id)
            .collect();

        let orphan_folder_previews: Vec<String> = all_folder_previews
            .into_iter()
            .filter(|path| {
                Self::is_on_accessible_drive(path, &accessible) && !Self::path_exists_fast(path)
            })
            .collect();

        if orphan_thumbs.is_empty() && orphan_folder_previews.is_empty() {
            log::debug!("[GC] No orphans found, skipping cleanup");
            return 0;
        }

        let mut removed = 0;
        // H-5: rusqlite Transaction — auto-rollback on error/panic
        let mut db = self.writer.lock();
        if let Ok(tx) = db.transaction() {
            if !orphan_thumbs.is_empty() {
                removed += Self::execute_batch_delete(&tx, CacheTable::Thumbnails, &orphan_thumbs);
            }
            if !orphan_folder_previews.is_empty() {
                removed += Self::execute_batch_delete(
                    &tx,
                    CacheTable::FolderPreviews,
                    &orphan_folder_previews,
                );
            }
            if let Err(e) = tx.commit() {
                log::error!("[GC] Full GC: transaction commit failed: {:?}", e);
            }
        }

        if removed > 0 {
            log::debug!(
                "[GC] Full GC removed {} entries (VACUUM not automatic)",
                removed
            );
        }
        removed
    }
}
