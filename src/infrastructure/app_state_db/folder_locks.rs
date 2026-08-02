use super::{AppStateDb, AppStateWriteError};
use crate::domain::file_entry::{FoldersPosition, GroupMode, SortMode, ViewMode};
use crate::domain::folder_lock::{FolderLock, FolderLockScope};
use rusqlite::params;
use std::collections::HashMap;
use std::time::Duration;

impl AppStateDb {
    /// Save a folder lock to the database. [WRITER]
    pub fn save_folder_lock(
        &self,
        path: &str,
        lock: &FolderLock,
    ) -> Result<(), AppStateWriteError> {
        let view_mode_str = lock.view_mode.preference_value();
        let sort_mode_str = match lock.sort_mode {
            SortMode::Name => "name",
            SortMode::Date => "date",
            SortMode::Size => "size",
            SortMode::Type => "type",
            SortMode::DriveTotalSpace => "drive_total",
            SortMode::DriveFreeSpace => "drive_free",
            SortMode::DriveLetter => "drive_letter",
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
        let scope_str = lock.scope.preference_value();
        let group_mode_str = lock.group_mode.preference_value();
        let group_desc_str = lock.group_descending.to_string();
        let mut db = self
            .writer
            .lock()
            .map_err(|_| AppStateWriteError::WriterLockPoisoned)?;
        Self::with_busy_timeout(&mut db, Duration::ZERO, |db| {
            db.execute(
                "INSERT OR REPLACE INTO folder_locks
                     (path, view_mode, sort_mode, sort_descending, folders_position,
                      group_mode, group_descending, scope)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    path,
                    view_mode_str,
                    sort_mode_str,
                    sort_desc_str,
                    folders_pos_str,
                    group_mode_str,
                    group_desc_str,
                    scope_str
                ],
            )?;
            Ok(())
        })?;
        log::info!(
            "[FOLDER-LOCK] Saved lock for {:?}: view={}, sort={}, desc={}, pos={}, scope={}",
            path,
            view_mode_str,
            sort_mode_str,
            sort_desc_str,
            folders_pos_str,
            scope_str,
        );
        Ok(())
    }

    /// Remove a folder lock. [WRITER]
    pub fn remove_folder_lock(&self, path: &str) -> Result<(), AppStateWriteError> {
        let mut db = self
            .writer
            .lock()
            .map_err(|_| AppStateWriteError::WriterLockPoisoned)?;
        Self::with_busy_timeout(&mut db, Duration::ZERO, |db| {
            db.execute("DELETE FROM folder_locks WHERE path = ?", params![path])?;
            Ok(())
        })?;
        Ok(())
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
            "SELECT path, view_mode, sort_mode, sort_descending, folders_position,
                    group_mode, group_descending, scope
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
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        });
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (
                    path,
                    view_mode_s,
                    sort_mode_s,
                    sort_desc_s,
                    folders_pos_s,
                    group_mode_s,
                    group_desc_s,
                    scope_s,
                ) = row;
                let view_mode = ViewMode::from_preference(&view_mode_s);
                let sort_mode = match sort_mode_s.as_str() {
                    "date" => SortMode::Date,
                    "size" => SortMode::Size,
                    "type" => SortMode::Type,
                    "drive_total" => SortMode::DriveTotalSpace,
                    "drive_free" => SortMode::DriveFreeSpace,
                    "drive_letter" => SortMode::DriveLetter,
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
                        group_mode: GroupMode::from_preference(&group_mode_s),
                        group_descending: group_desc_s == "true",
                        scope: FolderLockScope::from_preference(&scope_s),
                    },
                );
            }
        }
        log::info!(
            "[FOLDER-LOCK] Loaded {} folder locks from DB: {:?}",
            results.len(),
            results.keys().collect::<Vec<_>>()
        );
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_lock() -> FolderLock {
        FolderLock {
            view_mode: ViewMode::Grid,
            sort_mode: SortMode::Name,
            sort_descending: false,
            folders_position: FoldersPosition::First,
            group_mode: GroupMode::Type,
            group_descending: true,
            scope: FolderLockScope::CurrentFolder,
        }
    }

    #[test]
    fn failed_folder_lock_save_does_not_create_persisted_state() {
        let temp = tempfile::tempdir().unwrap();
        let db = AppStateDb::new(temp.path().to_path_buf()).unwrap();
        {
            let writer = db.writer.lock().unwrap();
            writer
                .execute_batch(
                    "CREATE TRIGGER fail_folder_lock_insert
                     BEFORE INSERT ON folder_locks
                     BEGIN
                         SELECT RAISE(FAIL, 'forced folder lock failure');
                     END;",
                )
                .unwrap();
        }

        assert!(db.save_folder_lock("C:\\Locked", &sample_lock()).is_err());
        assert!(db.get_all_folder_locks().is_empty());
    }

    #[test]
    fn failed_folder_lock_remove_preserves_persisted_state() {
        let temp = tempfile::tempdir().unwrap();
        let db = AppStateDb::new(temp.path().to_path_buf()).unwrap();
        db.save_folder_lock("C:\\Locked", &sample_lock()).unwrap();
        {
            let writer = db.writer.lock().unwrap();
            writer
                .execute_batch(
                    "CREATE TRIGGER fail_folder_lock_delete
                     BEFORE DELETE ON folder_locks
                     BEGIN
                         SELECT RAISE(FAIL, 'forced folder unlock failure');
                     END;",
                )
                .unwrap();
        }

        assert!(db.remove_folder_lock("C:\\Locked").is_err());
        assert!(db.get_all_folder_locks().contains_key("C:\\Locked"));
    }

    #[test]
    fn folder_lock_save_does_not_wait_for_external_sqlite_lock() {
        let temp = tempfile::tempdir().unwrap();
        let db = AppStateDb::new(temp.path().to_path_buf()).unwrap();
        let external = rusqlite::Connection::open(temp.path().join("app_state.db")).unwrap();
        external.execute("BEGIN IMMEDIATE", []).unwrap();
        let started = std::time::Instant::now();

        assert!(db.save_folder_lock("C:\\Locked", &sample_lock()).is_err());
        assert!(started.elapsed() < Duration::from_millis(100));
        external.execute("ROLLBACK", []).unwrap();
    }

    #[test]
    fn descendant_scope_round_trips() {
        let db = AppStateDb::new_in_memory().unwrap();
        let mut lock = sample_lock();
        lock.scope = FolderLockScope::Descendants;

        db.save_folder_lock("C:\\Locked", &lock).unwrap();

        let locks = db.get_all_folder_locks();
        assert_eq!(
            locks.get("C:\\Locked").unwrap().scope,
            FolderLockScope::Descendants
        );
        assert_eq!(locks.get("C:\\Locked").unwrap().group_mode, GroupMode::Type);
        assert!(locks.get("C:\\Locked").unwrap().group_descending);
    }

    #[test]
    fn migration_preserves_old_locks_as_current_folder_scope() {
        let db = AppStateDb::new_in_memory().unwrap();
        {
            let writer = db.writer.lock().unwrap();
            writer
                .execute_batch(
                    "DROP TABLE folder_locks;
                     CREATE TABLE folder_locks (
                         path TEXT PRIMARY KEY,
                         view_mode TEXT NOT NULL,
                         sort_mode TEXT NOT NULL,
                         sort_descending TEXT NOT NULL,
                         folders_position TEXT NOT NULL
                     );
                     INSERT INTO folder_locks
                         (path, view_mode, sort_mode, sort_descending, folders_position)
                     VALUES
                         ('C:\\OldLock', 'grid', 'name', 'false', 'first');",
                )
                .unwrap();
            AppStateDb::run_migrations(&writer);
        }

        let locks = db.get_all_folder_locks();
        assert_eq!(
            locks.get("C:\\OldLock").unwrap().scope,
            FolderLockScope::CurrentFolder
        );
        assert_eq!(
            locks.get("C:\\OldLock").unwrap().group_mode,
            GroupMode::None
        );
        assert!(!locks.get("C:\\OldLock").unwrap().group_descending);
    }
}
