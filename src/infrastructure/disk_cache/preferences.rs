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
