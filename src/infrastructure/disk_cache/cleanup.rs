use super::ThumbnailDiskCache;
use std::path::Path;

fn normalize_cache_path(path: &Path) -> String {
    let path_str = path.to_string_lossy();
    path_str
        .strip_prefix(r"\\?\")
        .unwrap_or(&path_str)
        .to_string()
}

impl ThumbnailDiskCache {
    /// Remove cache entries for a specific path (file or folder)
    /// [WRITER]
    pub fn remove_cache_for_path(&self, path: &Path) {
        let path_str = normalize_cache_path(path);

        if let Ok(mut db) = self.writer.lock() {
            let pattern = format!("{}\\%", path_str.trim_end_matches('\\'));

            // M-16: wrap all DELETEs in a single transaction — one fsync instead of multiple
            if let Ok(tx) = db.transaction() {
                let _ = tx.execute("DELETE FROM thumbnails WHERE path = ?", [&path_str]);
                let deleted = tx
                    .execute("DELETE FROM thumbnails WHERE path LIKE ?", [&pattern])
                    .unwrap_or(0);

                // Remove folder preview cache entries
                let _ = tx.execute(
                    "DELETE FROM folder_previews WHERE folder_path = ?",
                    [&path_str],
                );
                let _ = tx.execute(
                    "DELETE FROM folder_previews WHERE folder_path LIKE ?",
                    [&pattern],
                );

                let _ = tx.commit();

                // Log cleanup (VACUUM is not called here to avoid UI thread blocking;
                // it runs during garbage_collect() which is called at controlled times)
                if deleted > 0 {
                    log::debug!("[Cache] Cleaned {} entries for: {}", deleted, path_str);
                }
            }
        }
    }

    /// Rename a thumbnail cache entry from `old_path` to `new_path`.
    /// This preserves the cached thumbnail when a file is renamed, avoiding
    /// re-extraction on the next scroll-in.  Only the `thumbnails` table is
    /// updated; folder-preview entries are not affected (they are keyed on the
    /// folder path, not individual files).
    /// [WRITER]
    pub fn rename_cache_entry(&self, old_path: &Path, new_path: &Path) {
        let old_id = Self::hash_path(old_path);
        let new_id = Self::hash_path(new_path);
        let new_path_str = normalize_cache_path(new_path);

        if let Ok(db) = self.writer.lock() {
            // If a stale entry already exists under new_id (e.g. from a
            // previous run with the same name), remove it first to avoid a
            // UNIQUE constraint violation.
            let _ = db.execute("DELETE FROM thumbnails WHERE id = ?", [&new_id]);
            let updated = db
                .execute(
                    "UPDATE thumbnails SET id = ?, path = ? WHERE id = ?",
                    rusqlite::params![new_id, new_path_str, old_id],
                )
                .unwrap_or(0);
            if updated > 0 {
                log::debug!(
                    "[Cache] Renamed disk cache entry {:?} -> {:?}",
                    old_path.file_name().unwrap_or_default(),
                    new_path.file_name().unwrap_or_default()
                );
            }
        }
    }
}
