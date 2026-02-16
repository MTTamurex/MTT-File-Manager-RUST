use super::ThumbnailDiskCache;
use rusqlite::params;

impl ThumbnailDiskCache {
    /// Sets a user preference
    /// [WRITER]
    pub fn set_preference(&self, key: &str, value: &str) {
        if let Ok(db) = self.writer.lock() {
            let _ = db.execute(
                "INSERT OR REPLACE INTO user_preferences (key, value) VALUES (?, ?)",
                params![key, value],
            );
        }
    }

    /// Best-effort non-blocking batch preference write.
    /// Returns `true` when the batch was flushed, `false` when writer lock is busy.
    /// [WRITER]
    pub fn try_set_preferences_batch(&self, entries: &[(&str, String)]) -> bool {
        let mut db = match self.writer.try_lock() {
            Ok(db) => db,
            Err(_) => return false,
        };

        Self::write_preferences_batch(&mut db, entries);
        true
    }

    /// Blocking batch preference write.
    /// [WRITER]
    pub fn set_preferences_batch(&self, entries: &[(&str, String)]) {
        if let Ok(mut db) = self.writer.lock() {
            Self::write_preferences_batch(&mut db, entries);
        }
    }

    fn write_preferences_batch(db: &mut rusqlite::Connection, entries: &[(&str, String)]) {
        if entries.is_empty() {
            return;
        }

        if db.execute("BEGIN IMMEDIATE TRANSACTION", []).is_ok() {
            for (key, value) in entries {
                let _ = db.execute(
                    "INSERT OR REPLACE INTO user_preferences (key, value) VALUES (?, ?)",
                    params![key, value],
                );
            }
            let _ = db.execute("COMMIT", []);
        } else {
            for (key, value) in entries {
                let _ = db.execute(
                    "INSERT OR REPLACE INTO user_preferences (key, value) VALUES (?, ?)",
                    params![key, value],
                );
            }
        }
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
}
