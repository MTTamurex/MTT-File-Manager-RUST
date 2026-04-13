use super::AppStateDb;
use std::path::Path;

impl AppStateDb {
    /// Remove folder cover entries for a specific path (file or folder).
    /// Handles both the folder being deleted and files used as covers being deleted.
    /// [WRITER]
    pub fn remove_covers_for_path(&self, path: &Path) {
        let path_str = path.to_string_lossy().to_string();
        let path_str = path_str
            .strip_prefix(r"\\?\")
            .unwrap_or(&path_str)
            .to_string();

        if let Ok(mut db) = self.writer.lock() {
            let pattern = format!("{}\\%", path_str.trim_end_matches('\\'));

            if let Ok(tx) = db.transaction() {
                // Remove folder cover entries for the exact folder
                let _ = tx.execute(
                    "DELETE FROM folder_covers WHERE folder_path = ?",
                    [&path_str],
                );
                // Remove folder cover entries for children
                let _ = tx.execute(
                    "DELETE FROM folder_covers WHERE folder_path LIKE ?",
                    [&pattern],
                );
                // Exact match: this file IS a folder cover
                let _ = tx.execute(
                    "DELETE FROM folder_covers WHERE cover_path = ?",
                    [&path_str],
                );
                // Children match: covers inside a deleted folder
                let _ = tx.execute(
                    "DELETE FROM folder_covers WHERE cover_path LIKE ?",
                    [&pattern],
                );

                let _ = tx.commit();
            }
        }
    }
}
