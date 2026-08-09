use super::{AppStateDb, AppStateWriteError};
use rusqlite::ErrorCode;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::TryLockError;
use std::time::Duration;

#[derive(Debug)]
#[must_use]
pub enum FolderCoverReadOutcome {
    Completed(HashMap<PathBuf, PathBuf>),
    Busy,
    Failed,
}

#[derive(Debug)]
#[must_use]
pub enum FolderCoverRemoveOutcome {
    Removed,
    Busy,
    Failed(AppStateWriteError),
}

impl AppStateDb {
    /// Gets covers (thumbnails) for multiple folders at once
    /// [READER]
    /// PERFORMANCE: Uses chunking to stay within SQLite's parameter limit (999)
    pub fn get_folder_covers(&self, folder_paths: &[PathBuf]) -> HashMap<PathBuf, PathBuf> {
        if folder_paths.is_empty() {
            return HashMap::new();
        }

        let db = match self.reader.lock() {
            Ok(db) => db,
            Err(_) => return HashMap::new(),
        };

        query_folder_covers(&db, folder_paths).unwrap_or_else(|error| {
            log::warn!("[FOLDER-COVERS] Batch read failed: {error}");
            HashMap::new()
        })
    }

    /// Attempts a batch cover read without waiting for the reader or SQLite.
    /// An empty `Completed` map is a successful query, not lock contention.
    /// [READER - non-blocking]
    pub fn try_get_folder_covers(&self, folder_paths: &[PathBuf]) -> FolderCoverReadOutcome {
        if folder_paths.is_empty() {
            return FolderCoverReadOutcome::Completed(HashMap::new());
        }

        let mut db = match self.reader.try_lock() {
            Ok(db) => db,
            Err(TryLockError::WouldBlock) => return FolderCoverReadOutcome::Busy,
            Err(TryLockError::Poisoned(_)) => {
                log::error!("[FOLDER-COVERS] Reader lock is poisoned");
                return FolderCoverReadOutcome::Failed;
            }
        };

        match Self::with_busy_timeout(&mut db, Duration::ZERO, |db| {
            query_folder_covers(db, folder_paths)
        }) {
            Ok(covers) => FolderCoverReadOutcome::Completed(covers),
            Err(error) if is_sqlite_busy(&error) => FolderCoverReadOutcome::Busy,
            Err(error) => {
                log::warn!("[FOLDER-COVERS] Non-blocking batch read failed: {error}");
                FolderCoverReadOutcome::Failed
            }
        }
    }

    /// Remove a stored cover without waiting for the writer or SQLite.
    /// [WRITER - non-blocking]
    pub fn try_remove_folder_cover(&self, folder_path: &Path) -> FolderCoverRemoveOutcome {
        let mut db = match self.writer.try_lock() {
            Ok(db) => db,
            Err(TryLockError::WouldBlock) => return FolderCoverRemoveOutcome::Busy,
            Err(TryLockError::Poisoned(_)) => {
                return FolderCoverRemoveOutcome::Failed(AppStateWriteError::WriterLockPoisoned)
            }
        };

        match Self::with_busy_timeout(&mut db, Duration::ZERO, |db| {
            db.execute(
                "DELETE FROM folder_covers WHERE folder_path = ?",
                [folder_path.to_string_lossy()],
            )?;
            Ok(())
        }) {
            Ok(()) => FolderCoverRemoveOutcome::Removed,
            Err(error) if is_sqlite_busy(&error) => FolderCoverRemoveOutcome::Busy,
            Err(error) => FolderCoverRemoveOutcome::Failed(error.into()),
        }
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

    /// Non-blocking variant of `set_folder_cover`.
    /// Returns `true` if the write succeeded, `false` if the writer lock was busy.
    /// Use on the UI thread to avoid blocking when a worker holds the lock.
    /// [WRITER — non-blocking]
    pub fn try_set_folder_cover(&self, folder_path: &Path, cover_path: &Path) -> bool {
        match self.writer.try_lock() {
            Ok(db) => {
                let _ = db.execute(
                    "INSERT OR REPLACE INTO folder_covers (folder_path, cover_path) VALUES (?, ?)",
                    [folder_path.to_string_lossy(), cover_path.to_string_lossy()],
                );
                true
            }
            Err(_) => false,
        }
    }

    /// Remove the stored cover for a folder
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

fn query_folder_covers(
    db: &rusqlite::Connection,
    folder_paths: &[PathBuf],
) -> rusqlite::Result<HashMap<PathBuf, PathBuf>> {
    let mut results = HashMap::new();

    // SQLite parameter limit is 999, use 500 for safety margin.
    const BATCH_SIZE: usize = 500;

    for chunk in folder_paths.chunks(BATCH_SIZE) {
        let placeholders: Vec<&str> = chunk.iter().map(|_| "?").collect();
        let query = format!(
            "SELECT folder_path, cover_path FROM folder_covers WHERE folder_path IN ({})",
            placeholders.join(",")
        );

        let mut stmt = db.prepare(&query)?;
        let path_strs: Vec<String> = chunk
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        let rows = stmt.query_map(rusqlite::params_from_iter(path_strs.iter()), |row| {
            let folder_path: String = row.get(0)?;
            let cover_path: String = row.get(1)?;
            Ok((folder_path, cover_path))
        })?;
        for row in rows {
            let (folder_path, cover_path) = row?;
            // Skip path validation here: it can block on virtual/encrypted drives.
            results.insert(PathBuf::from(folder_path), PathBuf::from(cover_path));
        }
    }

    Ok(results)
}

fn is_sqlite_busy(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(sqlite_error, _)
            if matches!(sqlite_error.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

#[cfg(test)]
mod tests {
    use super::{AppStateDb, FolderCoverReadOutcome, FolderCoverRemoveOutcome};
    use std::path::{Path, PathBuf};

    #[test]
    fn try_get_folder_covers_distinguishes_completed_empty_from_busy() {
        let db = AppStateDb::new_in_memory().unwrap();
        let missing = vec![PathBuf::from(r"C:\missing")];
        let folder = PathBuf::from(r"C:\folder");
        let cover = PathBuf::from(r"C:\folder\cover.jpg");
        db.set_folder_cover(&folder, &cover);

        assert!(matches!(
            db.try_get_folder_covers(&missing),
            FolderCoverReadOutcome::Completed(covers) if covers.is_empty()
        ));

        let reader = db.reader.lock().unwrap();
        assert!(matches!(
            db.try_get_folder_covers(&missing),
            FolderCoverReadOutcome::Busy
        ));
        drop(reader);

        assert_eq!(
            db.get_folder_covers(std::slice::from_ref(&folder))
                .get(&folder),
            Some(&cover)
        );
    }

    #[test]
    fn try_remove_folder_cover_reports_busy_and_can_be_retried() {
        let db = AppStateDb::new_in_memory().unwrap();
        let folder = Path::new(r"C:\folder");
        let cover = Path::new(r"C:\folder\cover.jpg");
        db.set_folder_cover(folder, cover);

        let writer = db.writer.lock().unwrap();
        assert!(matches!(
            db.try_remove_folder_cover(folder),
            FolderCoverRemoveOutcome::Busy
        ));
        drop(writer);

        assert!(matches!(
            db.try_remove_folder_cover(folder),
            FolderCoverRemoveOutcome::Removed
        ));
        assert!(db.get_folder_covers(&[folder.to_path_buf()]).is_empty());
    }
}
