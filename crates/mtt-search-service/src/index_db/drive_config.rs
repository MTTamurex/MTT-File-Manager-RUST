use std::collections::HashMap;

use rusqlite::{params, Connection};

use super::IndexDb;

pub(super) fn create_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS drive_indexing_config (
            drive_letter TEXT PRIMARY KEY NOT NULL
                CHECK(length(drive_letter) = 1 AND drive_letter GLOB '[A-Z]'),
            enabled INTEGER NOT NULL CHECK(enabled IN (0, 1))
        );",
    )
    .map_err(|error| format!("Drive configuration schema error: {error}"))
}

impl IndexDb {
    pub(crate) fn load_drive_indexing_config(&self) -> Result<HashMap<char, bool>, String> {
        let conn = self.conn.lock();
        let mut statement = conn
            .prepare("SELECT drive_letter, enabled FROM drive_indexing_config")
            .map_err(|error| format!("Drive configuration read error: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|error| format!("Drive configuration read error: {error}"))?;
        let mut entries = HashMap::new();
        for row in rows {
            let (letter, enabled) =
                row.map_err(|error| format!("Drive configuration read error: {error}"))?;
            if letter.len() != 1
                || !letter.as_bytes()[0].is_ascii_uppercase()
                || !matches!(enabled, 0 | 1)
            {
                return Err("Invalid persisted drive configuration".into());
            }
            entries.insert(letter.as_bytes()[0] as char, enabled == 1);
        }
        Ok(entries)
    }

    pub(crate) fn set_drive_indexing(&self, letter: char, enabled: bool) -> Result<(), String> {
        if !letter.is_ascii_alphabetic() {
            return Err("drive letter must be an ASCII letter".into());
        }
        let letter = letter.to_ascii_uppercase().to_string();
        self.conn
            .lock()
            .execute(
                "INSERT INTO drive_indexing_config(drive_letter, enabled) VALUES (?1, ?2)
             ON CONFLICT(drive_letter) DO UPDATE SET enabled = excluded.enabled",
                params![letter, enabled],
            )
            .map_err(|error| format!("Drive configuration write error: {error}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn additive_schema_defaults_normalizes_and_survives_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("index.db");
        {
            let db = IndexDb::open(&path).unwrap();
            assert!(db.load_drive_indexing_config().unwrap().is_empty());
            db.set_drive_indexing('r', false).unwrap();
            db.set_drive_indexing('S', true).unwrap();
            assert!(db.set_drive_indexing('é', false).is_err());
        }
        let db = IndexDb::open(&path).unwrap();
        let config = db.load_drive_indexing_config().unwrap();
        assert_eq!(config.get(&'R'), Some(&false));
        assert_eq!(config.get(&'S'), Some(&true));
        assert_eq!(config.len(), 2);
        db.set_drive_indexing('R', true).unwrap();
        assert_eq!(
            db.load_drive_indexing_config().unwrap().get(&'R'),
            Some(&true)
        );
    }

    #[test]
    fn schema_rejects_noncanonical_values_and_read_failures_are_not_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let db = IndexDb::open(&directory.path().join("index.db")).unwrap();
        {
            let conn = db.conn.lock();
            for (letter, enabled) in [("rr", 0), ("r", 0), ("1", 0), ("R", 2)] {
                assert!(conn
                    .execute(
                        "INSERT INTO drive_indexing_config VALUES (?1, ?2)",
                        params![letter, enabled]
                    )
                    .is_err());
            }
            assert!(conn
                .execute("INSERT INTO drive_indexing_config VALUES (NULL, 0)", [])
                .is_err());
            conn.execute_batch("DROP TABLE drive_indexing_config")
                .unwrap();
        }
        assert!(db.load_drive_indexing_config().is_err());
        assert!(db.set_drive_indexing('R', false).is_err());
    }
}
