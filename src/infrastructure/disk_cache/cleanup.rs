use super::ThumbnailDiskCache;
use std::path::Path;

impl ThumbnailDiskCache {
    /// Remove cache entries for a specific path (file or folder)
    /// [WRITER]
    pub fn remove_cache_for_path(&self, path: &Path) {
        let path_str = path.to_string_lossy().to_string();
        let path_str = path_str
            .strip_prefix(r"\\?\")
            .unwrap_or(&path_str)
            .to_string();

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
}
