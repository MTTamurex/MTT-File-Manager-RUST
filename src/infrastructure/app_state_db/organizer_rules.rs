use super::AppStateDb;
use crate::domain::organizer_rule::{parse_extensions, OrganizerRule};
use rusqlite::params;
use std::path::PathBuf;

#[derive(Debug)]
pub enum OrganizerRuleDbError {
    DatabaseUnavailable,
    RuleNotFound,
    Database(String),
}

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

    pub fn save_organizer_rule(&self, rule: &OrganizerRule) -> Result<i64, OrganizerRuleDbError> {
        let db = self
            .writer
            .lock()
            .map_err(|_| OrganizerRuleDbError::DatabaseUnavailable)?;
        if rule.id == 0 {
            db.execute(
                "INSERT INTO organizer_rules (source_folder, destination_folder, extensions, enabled) VALUES (?1, ?2, ?3, ?4)",
                params![rule.source_folder.to_string_lossy(), rule.destination_folder.to_string_lossy(), rule.extensions_csv(), rule.enabled],
            ).map_err(|error| OrganizerRuleDbError::Database(error.to_string()))?;
            Ok(db.last_insert_rowid())
        } else {
            let updated = db.execute(
                "UPDATE organizer_rules SET source_folder = ?1, destination_folder = ?2, extensions = ?3, enabled = ?4 WHERE id = ?5",
                params![rule.source_folder.to_string_lossy(), rule.destination_folder.to_string_lossy(), rule.extensions_csv(), rule.enabled, rule.id],
            ).map_err(|error| OrganizerRuleDbError::Database(error.to_string()))?;
            if updated == 0 {
                return Err(OrganizerRuleDbError::RuleNotFound);
            }
            Ok(rule.id)
        }
    }

    pub fn delete_organizer_rule(&self, id: i64) -> Result<(), OrganizerRuleDbError> {
        let db = self
            .writer
            .lock()
            .map_err(|_| OrganizerRuleDbError::DatabaseUnavailable)?;
        let deleted = db
            .execute("DELETE FROM organizer_rules WHERE id = ?1", params![id])
            .map_err(|error| OrganizerRuleDbError::Database(error.to_string()))?;
        if deleted == 0 {
            Err(OrganizerRuleDbError::RuleNotFound)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: i64, source: PathBuf, destination: PathBuf) -> OrganizerRule {
        OrganizerRule::new(id, source, destination, vec!["txt".to_string()], true)
            .expect("valid organizer rule")
    }

    #[test]
    fn updating_or_deleting_a_missing_rule_returns_not_found() {
        let state_dir = tempfile::tempdir().expect("state directory");
        let source = tempfile::tempdir().expect("source directory");
        let destination = tempfile::tempdir().expect("destination directory");
        let db = AppStateDb::new(state_dir.path().to_path_buf()).expect("database");
        let missing_rule = rule(
            42,
            source.path().to_path_buf(),
            destination.path().to_path_buf(),
        );

        assert!(matches!(
            db.save_organizer_rule(&missing_rule),
            Err(OrganizerRuleDbError::RuleNotFound)
        ));
        assert!(matches!(
            db.delete_organizer_rule(missing_rule.id),
            Err(OrganizerRuleDbError::RuleNotFound)
        ));
    }
}
