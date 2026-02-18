use super::ThumbnailDiskCache;
use crate::domain::file_entry::{FoldersPosition, SortMode, ViewMode};
use crate::domain::folder_lock::FolderLock;
use rusqlite::params;
use std::collections::HashMap;

impl ThumbnailDiskCache {
    /// Save a folder lock to the database. [WRITER]
    pub fn save_folder_lock(&self, path: &str, lock: &FolderLock) {
        let view_mode_str = match lock.view_mode {
            ViewMode::Grid => "grid",
            ViewMode::List => "list",
        };
        let sort_mode_str = match lock.sort_mode {
            SortMode::Name => "name",
            SortMode::Date => "date",
            SortMode::Size => "size",
            SortMode::Type => "type",
            SortMode::DriveTotalSpace => "drive_total",
            SortMode::DriveFreeSpace => "drive_free",
        };
        let sort_desc_str = if lock.sort_descending {
            "true"
        } else {
            "false"
        };
        let folders_pos_str = match lock.folders_position {
            FoldersPosition::First => "first",
            FoldersPosition::Last => "last",
            FoldersPosition::Mixed => "mixed",
        };
        if let Ok(db) = self.writer.lock() {
            match db.execute(
                "INSERT OR REPLACE INTO folder_locks
                 (path, view_mode, sort_mode, sort_descending, folders_position)
                 VALUES (?, ?, ?, ?, ?)",
                params![
                    path,
                    view_mode_str,
                    sort_mode_str,
                    sort_desc_str,
                    folders_pos_str
                ],
            ) {
                Ok(_) => log::info!("[FOLDER-LOCK] Saved lock for {:?}: view={}, sort={}, desc={}, pos={}", path, view_mode_str, sort_mode_str, sort_desc_str, folders_pos_str),
                Err(e) => log::error!("[FOLDER-LOCK] Failed to save lock for {:?}: {:?}", path, e),
            }
        } else {
            log::error!("[FOLDER-LOCK] Failed to acquire writer lock for save_folder_lock");
        }
    }

    /// Remove a folder lock. [WRITER]
    pub fn remove_folder_lock(&self, path: &str) {
        if let Ok(db) = self.writer.lock() {
            let _ = db.execute("DELETE FROM folder_locks WHERE path = ?", params![path]);
        }
    }

    /// Load all folder locks at startup. [READER]
    pub fn get_all_folder_locks(&self) -> HashMap<String, FolderLock> {
        let mut results = HashMap::new();
        let db = match self.reader.lock() {
            Ok(db) => db,
            Err(e) => {
                log::error!("[FOLDER-LOCK] Failed to acquire reader lock: {:?}", e);
                return results;
            }
        };
        let mut stmt = match db.prepare(
            "SELECT path, view_mode, sort_mode, sort_descending, folders_position
             FROM folder_locks",
        ) {
            Ok(s) => s,
            Err(e) => {
                log::error!("[FOLDER-LOCK] Failed to prepare SELECT statement: {:?}", e);
                return results;
            }
        };
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        });
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (path, view_mode_s, sort_mode_s, sort_desc_s, folders_pos_s) = row;
                let view_mode = match view_mode_s.as_str() {
                    "list" => ViewMode::List,
                    _ => ViewMode::Grid,
                };
                let sort_mode = match sort_mode_s.as_str() {
                    "date" => SortMode::Date,
                    "size" => SortMode::Size,
                    "type" => SortMode::Type,
                    "drive_total" => SortMode::DriveTotalSpace,
                    "drive_free" => SortMode::DriveFreeSpace,
                    _ => SortMode::Name,
                };
                let sort_descending = sort_desc_s == "true";
                let folders_position = match folders_pos_s.as_str() {
                    "last" => FoldersPosition::Last,
                    "mixed" => FoldersPosition::Mixed,
                    _ => FoldersPosition::First,
                };
                results.insert(
                    path,
                    FolderLock {
                        view_mode,
                        sort_mode,
                        sort_descending,
                        folders_position,
                    },
                );
            }
        }
        log::info!("[FOLDER-LOCK] Loaded {} folder locks from DB: {:?}", results.len(), results.keys().collect::<Vec<_>>());
        results
    }
}
