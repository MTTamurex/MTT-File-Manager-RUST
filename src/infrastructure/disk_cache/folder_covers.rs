use super::ThumbnailDiskCache;
use std::path::{Path, PathBuf};

impl ThumbnailDiskCache {
    /// Gets covers (thumbnails) for multiple folders at once
    /// [READER]
    /// PERFORMANCE: Uses chunking to stay within SQLite's parameter limit (999)
    pub fn get_folder_covers(
        &self,
        folder_paths: &[PathBuf],
    ) -> std::collections::HashMap<PathBuf, PathBuf> {
        let mut results = std::collections::HashMap::new();
        let mut stale_folder_rows = Vec::new();
        if folder_paths.is_empty() {
            return results;
        }

        // SQLite parameter limit is 999, use 500 for safety margin
        const BATCH_SIZE: usize = 500;

        {
            let db = match self.reader.lock() {
                Ok(db) => db,
                Err(_) => return results,
            };

            for chunk in folder_paths.chunks(BATCH_SIZE) {
                let placeholders: Vec<&str> = chunk.iter().map(|_| "?").collect();
                let query = format!(
                    "SELECT folder_path, cover_path FROM folder_covers WHERE folder_path IN ({})",
                    placeholders.join(",")
                );

                if let Ok(mut stmt) = db.prepare(&query) {
                    let path_strs: Vec<String> = chunk
                        .iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect();

                    if let Ok(rows) =
                        stmt.query_map(rusqlite::params_from_iter(path_strs.iter()), |row| {
                            let f_path: String = row.get(0)?;
                            let c_path: String = row.get(1)?;
                            Ok((f_path, c_path))
                        })
                    {
                        for row in rows.flatten() {
                            if Self::path_exists_fast(&row.1) {
                                results.insert(PathBuf::from(row.0), PathBuf::from(row.1));
                            } else {
                                stale_folder_rows.push(PathBuf::from(row.0));
                            }
                        }
                    }
                }
            }
        }

        // Best-effort stale cleanup after releasing reader lock.
        // This prevents old/non-existent cover paths from reappearing.
        for folder in stale_folder_rows {
            self.remove_folder_cover(&folder);
        }

        results
    }

    /// Saves the discovered cover (thumbnail) for a folder
    /// [WRITER]
    pub fn set_folder_cover(&self, folder_path: &Path, cover_path: &Path) {
        if let Ok(db) = self.writer.lock() {
            let _ = db.execute(
                "INSERT OR REPLACE INTO folder_covers (folder_path, cover_path) VALUES (?, ?)",
                [folder_path.to_string_lossy(), cover_path.to_string_lossy()],
            );
        }
    }

    /// Remove a capa armazenada de uma pasta
    /// [WRITER]
    pub fn remove_folder_cover(&self, folder_path: &Path) {
        if let Ok(db) = self.writer.lock() {
            let _ = db.execute(
                "DELETE FROM folder_covers WHERE folder_path = ?",
                [folder_path.to_string_lossy()],
            );
        }
    }
}
