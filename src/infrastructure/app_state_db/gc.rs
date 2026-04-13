use super::AppStateDb;
use rusqlite::params;
use std::path::Path;

impl AppStateDb {
    fn path_exists_fast(path: &str) -> bool {
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

    fn is_on_accessible_drive(
        path: &str,
        accessible: &std::collections::HashSet<String>,
    ) -> bool {
        match Self::extract_drive_root(path) {
            Some(root) => accessible.contains(&root),
            None => true,
        }
    }

    /// Incremental GC for folder_covers: scans bounded sample of folder paths,
    /// removes entries whose folder no longer exists on an accessible drive.
    /// [WRITER]
    pub fn garbage_collect_covers_incremental(&self, max_candidates: usize) -> usize {
        let limit = max_candidates.max(1) as i64;

        let sampled_folders: Vec<String> = {
            let db = match self.writer.lock() {
                Ok(db) => db,
                Err(_) => {
                    log::warn!("[GC-State] Incremental covers pass skipped: writer lock failed");
                    return 0;
                }
            };

            db.prepare(
                "SELECT folder_path FROM folder_covers \
                 WHERE rowid >= (ABS(RANDOM()) % MAX((SELECT COALESCE(MAX(rowid),0)+1 FROM folder_covers),1)) \
                 LIMIT ?1",
            )
            .and_then(|mut stmt| {
                stmt.query_map(params![limit], |row| row.get::<_, String>(0))
                    .map(|rows| rows.flatten().collect())
            })
            .unwrap_or_default()
        };

        if sampled_folders.is_empty() {
            return 0;
        }

        let accessible = Self::accessible_drives(sampled_folders.iter().map(|p| p.as_str()));

        let orphans: Vec<String> = sampled_folders
            .into_iter()
            .filter(|path| {
                Self::is_on_accessible_drive(path, &accessible) && !Self::path_exists_fast(path)
            })
            .collect();

        if orphans.is_empty() {
            return 0;
        }

        let mut removed = 0;
        if let Ok(mut db) = self.writer.lock() {
            if let Ok(tx) = db.transaction() {
                const BATCH_SIZE: usize = 500;
                for chunk in orphans.chunks(BATCH_SIZE) {
                    let placeholders = std::iter::repeat_n("?", chunk.len())
                        .collect::<Vec<_>>()
                        .join(",");
                    let sql = format!(
                        "DELETE FROM folder_covers WHERE folder_path IN ({})",
                        placeholders
                    );
                    match tx.execute(&sql, rusqlite::params_from_iter(chunk.iter())) {
                        Ok(c) => removed += c,
                        Err(e) => {
                            log::error!("[GC-State] Failed to delete covers batch: {:?}", e)
                        }
                    }
                }
                if let Err(e) = tx.commit() {
                    log::error!(
                        "[GC-State] Incremental covers: transaction commit failed: {:?}",
                        e
                    );
                }
            }
        }

        if removed > 0 {
            log::debug!(
                "[GC-State] Incremental covers pass removed {} entries",
                removed
            );
        }
        removed
    }
}
