use super::AppStateDb;
use crate::domain::organizer_rule::{parse_extensions, OrganizerRule};
use rusqlite::params;
use std::path::PathBuf;

impl AppStateDb {
    pub fn get_organizer_rules(&self) -> Vec<OrganizerRule> {
        let Ok(db) = self.reader.lock() else {
            return Vec::new();
        };
        let Ok(mut statement) = db.prepare(
            "SELECT id, source_folder, destination_folder, extensions, enabled FROM organizer_rules ORDER BY id ASC",
        ) else { return Vec::new(); };
        let Ok(rows) = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, bool>(4)?,
            ))
        }) else {
            return Vec::new();
        };
        rows.flatten()
            .filter_map(|(id, source, destination, extensions, enabled)| {
                OrganizerRule::from_persisted(
                    id,
                    PathBuf::from(source),
                    PathBuf::from(destination),
                    parse_extensions(&extensions).ok()?,
                    enabled,
                )
                .ok()
            })
            .collect()
    }

    pub fn save_organizer_rule(&self, rule: &OrganizerRule) -> Result<i64, String> {
        let db = self
            .writer
            .lock()
            .map_err(|_| "Banco de dados indisponível")?;
        if rule.id == 0 {
            db.execute(
                "INSERT INTO organizer_rules (source_folder, destination_folder, extensions, enabled) VALUES (?1, ?2, ?3, ?4)",
                params![rule.source_folder.to_string_lossy(), rule.destination_folder.to_string_lossy(), rule.extensions_csv(), rule.enabled],
            ).map_err(|error| error.to_string())?;
            Ok(db.last_insert_rowid())
        } else {
            db.execute(
                "UPDATE organizer_rules SET source_folder = ?1, destination_folder = ?2, extensions = ?3, enabled = ?4 WHERE id = ?5",
                params![rule.source_folder.to_string_lossy(), rule.destination_folder.to_string_lossy(), rule.extensions_csv(), rule.enabled, rule.id],
            ).map_err(|error| error.to_string())?;
            Ok(rule.id)
        }
    }

    pub fn delete_organizer_rule(&self, id: i64) {
        if let Ok(db) = self.writer.lock() {
            let _ = db.execute("DELETE FROM organizer_rules WHERE id = ?1", params![id]);
        }
    }
}
