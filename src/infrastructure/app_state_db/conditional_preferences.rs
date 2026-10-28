use super::{preferences::is_sqlite_busy, AppStateDb, AppStateWriteError, PreferenceWriteOutcome};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use std::{
    sync::TryLockError,
    time::{Duration, Instant},
};

impl AppStateDb {
    /// Compare and replace under the same cross-process SQLite transaction.
    /// The boolean reports whether this transaction replaced the stored value.
    pub(crate) fn update_preference_if(
        &self,
        key: &str,
        value: &str,
        timeout: Duration,
        accept: impl FnOnce(Option<&str>) -> bool,
    ) -> (PreferenceWriteOutcome, bool) {
        let deadline = Instant::now() + timeout;
        let mut db = loop {
            match self.writer.try_lock() {
                Ok(db) => break db,
                Err(TryLockError::Poisoned(_)) => {
                    return (
                        PreferenceWriteOutcome::Failed(AppStateWriteError::WriterLockPoisoned),
                        false,
                    )
                }
                Err(TryLockError::WouldBlock) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return (PreferenceWriteOutcome::Busy, false);
                    }
                    std::thread::sleep(remaining.min(Duration::from_millis(5)));
                }
            }
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return (PreferenceWriteOutcome::Busy, false);
        }
        let result = Self::with_busy_timeout(&mut db, remaining, |db| {
            let transaction = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let previous: Option<String> = transaction
                .query_row(
                    "SELECT value FROM user_preferences WHERE key = ?",
                    [key],
                    |row| row.get(0),
                )
                .optional()?;
            let applied = accept(previous.as_deref());
            if applied {
                transaction.execute(
                    "INSERT OR REPLACE INTO user_preferences (key, value) VALUES (?, ?)",
                    params![key, value],
                )?;
            }
            transaction.commit()?;
            Ok(applied)
        });
        match result {
            Ok(applied) => (PreferenceWriteOutcome::Persisted, applied),
            Err(error) if is_sqlite_busy(&error) => (PreferenceWriteOutcome::Busy, false),
            Err(error) => (PreferenceWriteOutcome::Failed(error.into()), false),
        }
    }
}
