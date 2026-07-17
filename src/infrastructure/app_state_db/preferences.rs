use super::{AppStateDb, AppStateWriteError};
use rusqlite::{params, ErrorCode, TransactionBehavior};
use std::sync::TryLockError;
use std::time::Duration;

const BLOCKING_WRITE_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Debug)]
#[must_use]
pub enum PreferenceWriteOutcome {
    Persisted,
    Busy,
    Failed(AppStateWriteError),
}

impl AppStateDb {
    /// Sets a user preference
    /// [WRITER]
    pub fn set_preference(&self, key: &str, value: &str) -> Result<(), AppStateWriteError> {
        let mut db = self
            .writer
            .lock()
            .map_err(|_| AppStateWriteError::WriterLockPoisoned)?;
        Self::with_busy_timeout(&mut db, BLOCKING_WRITE_TIMEOUT, |db| {
            Self::write_preference(db, key, value)
        })?;
        Ok(())
    }

    pub fn try_set_preference(&self, key: &str, value: &str) -> PreferenceWriteOutcome {
        let mut db = match self.writer.try_lock() {
            Ok(db) => db,
            Err(TryLockError::WouldBlock) => return PreferenceWriteOutcome::Busy,
            Err(TryLockError::Poisoned(_)) => {
                return PreferenceWriteOutcome::Failed(AppStateWriteError::WriterLockPoisoned)
            }
        };

        match Self::with_busy_timeout(&mut db, Duration::ZERO, |db| {
            Self::write_preference(db, key, value)
        }) {
            Ok(()) => PreferenceWriteOutcome::Persisted,
            Err(error) if is_sqlite_busy(&error) => PreferenceWriteOutcome::Busy,
            Err(error) => PreferenceWriteOutcome::Failed(error.into()),
        }
    }

    /// Non-blocking batch preference write.
    /// [WRITER]
    pub fn try_set_preferences_batch(&self, entries: &[(&str, String)]) -> PreferenceWriteOutcome {
        let mut db = match self.writer.try_lock() {
            Ok(db) => db,
            Err(TryLockError::WouldBlock) => return PreferenceWriteOutcome::Busy,
            Err(TryLockError::Poisoned(_)) => {
                return PreferenceWriteOutcome::Failed(AppStateWriteError::WriterLockPoisoned)
            }
        };

        match Self::write_preferences_batch_with_timeout(&mut db, entries, Duration::ZERO) {
            Ok(()) => PreferenceWriteOutcome::Persisted,
            Err(error) if is_sqlite_busy(&error) => PreferenceWriteOutcome::Busy,
            Err(error) => PreferenceWriteOutcome::Failed(error.into()),
        }
    }

    /// Blocking batch preference write.
    /// [WRITER]
    pub fn set_preferences_batch(
        &self,
        entries: &[(&str, String)],
    ) -> Result<(), AppStateWriteError> {
        let mut db = self
            .writer
            .lock()
            .map_err(|_| AppStateWriteError::WriterLockPoisoned)?;
        Self::write_preferences_batch_with_timeout(&mut db, entries, BLOCKING_WRITE_TIMEOUT)?;
        Ok(())
    }

    fn write_preferences_batch_with_timeout(
        db: &mut rusqlite::Connection,
        entries: &[(&str, String)],
        timeout: Duration,
    ) -> rusqlite::Result<()> {
        Self::with_busy_timeout(db, timeout, |db| Self::write_preferences_batch(db, entries))
    }

    pub(super) fn with_busy_timeout<T>(
        db: &mut rusqlite::Connection,
        timeout: Duration,
        operation: impl FnOnce(&mut rusqlite::Connection) -> rusqlite::Result<T>,
    ) -> rusqlite::Result<T> {
        let previous_timeout_ms: u64 = db.query_row("PRAGMA busy_timeout", [], |row| row.get(0))?;
        db.busy_timeout(timeout)?;
        let operation_result = operation(db);
        let restore_result = db.busy_timeout(Duration::from_millis(previous_timeout_ms));

        match (operation_result, restore_result) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Ok(value), Ok(())) => Ok(value),
        }
    }

    fn write_preference(db: &rusqlite::Connection, key: &str, value: &str) -> rusqlite::Result<()> {
        db.execute(
            "INSERT OR REPLACE INTO user_preferences (key, value) VALUES (?, ?)",
            params![key, value],
        )?;
        Ok(())
    }

    fn write_preferences_batch(
        db: &mut rusqlite::Connection,
        entries: &[(&str, String)],
    ) -> rusqlite::Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let mut statement = tx.prepare_cached(
                "INSERT OR REPLACE INTO user_preferences (key, value) VALUES (?, ?)",
            )?;
            for (key, value) in entries {
                statement.execute(params![key, value])?;
            }
        }
        tx.commit()
    }

    /// Gets a user preference
    /// [READER]
    pub fn get_preference(&self, key: &str) -> Option<String> {
        if let Ok(db) = self.reader.lock() {
            let mut stmt = db
                .prepare("SELECT value FROM user_preferences WHERE key = ?")
                .ok()?;
            stmt.query_row(params![key], |row| row.get(0)).ok()
        } else {
            None
        }
    }

    /// Loads all user preferences in a single query.
    /// [READER]
    pub fn get_all_preferences(&self) -> std::collections::HashMap<String, String> {
        let mut map = std::collections::HashMap::new();
        if let Ok(db) = self.reader.lock() {
            if let Ok(mut stmt) = db.prepare("SELECT key, value FROM user_preferences") {
                if let Ok(rows) = stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                }) {
                    for row in rows.flatten() {
                        map.insert(row.0, row.1);
                    }
                }
            }
        }
        map
    }
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
    use super::{AppStateDb, PreferenceWriteOutcome};
    use std::time::Duration;

    #[test]
    fn preference_batch_rolls_back_when_any_entry_fails() {
        let temp = tempfile::tempdir().unwrap();
        let db = AppStateDb::new(temp.path().to_path_buf()).unwrap();
        db.set_preference("first", "old").unwrap();
        {
            let writer = db.writer.lock().unwrap();
            writer
                .execute_batch(
                    "CREATE TRIGGER fail_bad_preference
                     BEFORE INSERT ON user_preferences
                     WHEN NEW.key = 'bad'
                     BEGIN
                         SELECT RAISE(FAIL, 'forced preference failure');
                     END;",
                )
                .unwrap();
        }

        let entries = [("first", "new".to_string()), ("bad", "value".to_string())];
        assert!(matches!(
            db.try_set_preferences_batch(&entries),
            PreferenceWriteOutcome::Failed(_)
        ));
        assert_eq!(db.get_preference("first").as_deref(), Some("old"));
        assert_eq!(db.get_preference("bad"), None);
    }

    #[test]
    fn nonblocking_preference_batch_reports_busy_writer() {
        let temp = tempfile::tempdir().unwrap();
        let db = AppStateDb::new(temp.path().to_path_buf()).unwrap();
        let _writer = db.writer.lock().unwrap();
        let entries = [("key", "value".to_string())];

        assert!(matches!(
            db.try_set_preferences_batch(&entries),
            PreferenceWriteOutcome::Busy
        ));
    }

    #[test]
    fn nonblocking_preference_batch_does_not_wait_for_external_sqlite_lock() {
        let temp = tempfile::tempdir().unwrap();
        let db = AppStateDb::new(temp.path().to_path_buf()).unwrap();
        let timeout_before: u64 = db
            .writer
            .lock()
            .unwrap()
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        let external = rusqlite::Connection::open(temp.path().join("app_state.db")).unwrap();
        external.execute("BEGIN IMMEDIATE", []).unwrap();
        let entries = [("key", "value".to_string())];
        let started = std::time::Instant::now();

        assert!(matches!(
            db.try_set_preferences_batch(&entries),
            PreferenceWriteOutcome::Busy
        ));
        assert!(started.elapsed() < Duration::from_millis(100));
        let timeout_after: u64 = db
            .writer
            .lock()
            .unwrap()
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(timeout_after, timeout_before);
        external.execute("ROLLBACK", []).unwrap();
    }

    #[test]
    fn blocking_preference_batch_uses_bounded_sqlite_wait() {
        let temp = tempfile::tempdir().unwrap();
        let db = AppStateDb::new(temp.path().to_path_buf()).unwrap();
        let external = rusqlite::Connection::open(temp.path().join("app_state.db")).unwrap();
        external.execute("BEGIN IMMEDIATE", []).unwrap();
        let entries = [("key", "value".to_string())];
        let started = std::time::Instant::now();

        assert!(db.set_preferences_batch(&entries).is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        external.execute("ROLLBACK", []).unwrap();
    }
}
