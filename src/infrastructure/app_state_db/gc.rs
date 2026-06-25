use super::AppStateDb;
use rusqlite::params;
use std::path::{Path, PathBuf};

impl AppStateDb {
    fn path_exists_fast(path: &str) -> bool {
        crate::infrastructure::onedrive::fast_path_exists(Path::new(path))
    }

    /// Extract an accessible filesystem root (e.g., "X:\\" or "\\\\server\\share").
    fn extract_drive_root(path: &str) -> Option<String> {
        let normalized = crate::domain::file_tag::normalize_tag_path_text(path);
        let bytes = normalized.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\'
        {
            return Some(format!("{}:\\", bytes[0] as char));
        }

        if let Some(rest) = normalized.strip_prefix("\\\\") {
            let mut parts = rest.split('\\');
            let server = parts.next().filter(|part| !part.is_empty())?;
            let share = parts.next().filter(|part| !part.is_empty())?;
            return Some(format!("\\\\{}\\{}", server, share));
        }

        None
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

    fn is_on_accessible_drive(path: &str, accessible: &std::collections::HashSet<String>) -> bool {
        match Self::extract_drive_root(path) {
            Some(root) => accessible.contains(&root),
            None => false,
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

    /// Incremental GC for file tag assignments. Removes assignments whose path no
    /// longer exists on an accessible drive. [WRITER]
    pub fn garbage_collect_tag_assignments_incremental(
        &self,
        max_candidates: usize,
    ) -> (usize, Vec<PathBuf>) {
        let limit = max_candidates.max(1) as i64;

        let sampled_paths: Vec<String> = {
            let db = match self.writer.lock() {
                Ok(db) => db,
                Err(_) => {
                    log::warn!("[GC-State] Incremental tag pass skipped: writer lock failed");
                    return (0, Vec::new());
                }
            };

            db.prepare(
                "SELECT file_path FROM file_tag_assignments \
                 WHERE rowid >= (ABS(RANDOM()) % MAX((SELECT COALESCE(MAX(rowid),0)+1 FROM file_tag_assignments),1)) \
                 LIMIT ?1",
            )
            .and_then(|mut stmt| {
                stmt.query_map(params![limit], |row| row.get::<_, String>(0))
                    .map(|rows| rows.flatten().collect())
            })
            .unwrap_or_default()
        };

        if sampled_paths.is_empty() {
            return (0, Vec::new());
        }

        let accessible = Self::accessible_drives(sampled_paths.iter().map(|p| p.as_str()));
        let orphans: Vec<String> = sampled_paths
            .into_iter()
            .filter(|path| {
                Self::is_on_accessible_drive(path, &accessible) && !Self::path_exists_fast(path)
            })
            .collect();

        if orphans.is_empty() {
            return (0, Vec::new());
        }

        let removed_result = (|| -> rusqlite::Result<usize> {
            let mut db = self
                .writer
                .lock()
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
            let tx = db.transaction()?;
            let mut removed = 0;
            const BATCH_SIZE: usize = 500;
            for chunk in orphans.chunks(BATCH_SIZE) {
                let placeholders = std::iter::repeat_n("?", chunk.len())
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "DELETE FROM file_tag_assignments WHERE file_path IN ({})",
                    placeholders
                );
                removed += tx.execute(&sql, rusqlite::params_from_iter(chunk.iter()))?;
            }
            tx.commit()?;
            Ok(removed)
        })();

        let removed = match removed_result {
            Ok(removed) => removed,
            Err(e) => {
                log::error!("[GC-State] Incremental tags delete failed: {:?}", e);
                return (0, Vec::new());
            }
        };

        if removed > 0 {
            log::debug!(
                "[GC-State] Incremental tags pass removed {} assignments",
                removed
            );
        }

        let mut seen_paths = std::collections::HashSet::new();
        let removed_paths = orphans
            .into_iter()
            .filter(|path| seen_paths.insert(path.to_lowercase()))
            .map(PathBuf::from)
            .collect();
        (removed, removed_paths)
    }
}
