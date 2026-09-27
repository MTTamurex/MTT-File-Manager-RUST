use super::AppStateDb;
use crate::domain::organizer_rule::{
    find_extension_conflict, parse_extensions, OrganizerConflictPolicy, OrganizerRule,
    OrganizerRuleError,
};
use rusqlite::{params, Connection};
use std::path::PathBuf;

#[derive(Debug)]
pub enum OrganizerRuleDbError {
    DatabaseUnavailable,
    RuleNotFound,
    Database(String),
    ExtensionConflict {
        extension: String,
        destination: String,
    },
}

impl AppStateDb {
    pub fn get_organizer_rules(&self) -> Vec<OrganizerRule> {
        let Ok(db) = self.reader.lock() else {
            return Vec::new();
        };
        load_rules_from_connection(&db).unwrap_or_default()
    }

    pub fn save_organizer_rule(&self, rule: &OrganizerRule) -> Result<i64, OrganizerRuleDbError> {
        let db = self
            .writer
            .lock()
            .map_err(|_| OrganizerRuleDbError::DatabaseUnavailable)?;
        let stored_rules =
            load_rules_from_connection(&db).map_err(OrganizerRuleDbError::Database)?;
        if let Some(conflict) = find_extension_conflict(&stored_rules, rule) {
            return Err(OrganizerRuleDbError::ExtensionConflict {
                extension: conflict.extension,
                destination: conflict.existing_destination.to_string_lossy().into_owned(),
            });
        }
        let conflict_policy = rule.conflict_policy.storage_key();
        let conflict_folder = rule
            .conflict_policy
            .conflict_folder()
            .map(|path| path.to_string_lossy().into_owned());
        if rule.id == 0 {
            db.execute(
                "INSERT INTO organizer_rules
                    (source_folder, destination_folder, extensions, enabled,
                     conflict_policy, conflict_folder)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    rule.source_folder.to_string_lossy(),
                    rule.destination_folder.to_string_lossy(),
                    rule.extensions_csv(),
                    rule.enabled,
                    conflict_policy,
                    conflict_folder,
                ],
            )
            .map_err(|error| OrganizerRuleDbError::Database(error.to_string()))?;
            Ok(db.last_insert_rowid())
        } else {
            let updated = db
                .execute(
                    "UPDATE organizer_rules
                 SET source_folder = ?1, destination_folder = ?2, extensions = ?3,
                     enabled = ?4, conflict_policy = ?5, conflict_folder = ?6
                 WHERE id = ?7",
                    params![
                        rule.source_folder.to_string_lossy(),
                        rule.destination_folder.to_string_lossy(),
                        rule.extensions_csv(),
                        rule.enabled,
                        conflict_policy,
                        conflict_folder,
                        rule.id,
                    ],
                )
                .map_err(|error| OrganizerRuleDbError::Database(error.to_string()))?;
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

fn load_rules_from_connection(connection: &Connection) -> Result<Vec<OrganizerRule>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, source_folder, destination_folder, extensions, enabled,
                    conflict_policy, conflict_folder
             FROM organizer_rules ORDER BY id ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, bool>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let rows: Vec<_> = rows
        .collect::<Result<_, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows
        .into_iter()
        .filter_map(
            |(id, source, destination, extensions, enabled, policy, conflict_folder)| {
                let conflict_policy = OrganizerConflictPolicy::from_persisted(
                    &policy,
                    conflict_folder.map(PathBuf::from),
                );
                let source = PathBuf::from(source);
                let destination = PathBuf::from(destination);
                let extensions = parse_extensions(&extensions).ok()?;
                match OrganizerRule::from_persisted_with_policy(
                    id,
                    source.clone(),
                    destination.clone(),
                    extensions.clone(),
                    enabled,
                    conflict_policy,
                ) {
                    Ok(rule) => Some(rule),
                    Err(OrganizerRuleError::InvalidConflictFolder) => {
                        OrganizerRule::from_persisted(id, source, destination, extensions, enabled)
                            .ok()
                    }
                    Err(_) => None,
                }
            },
        )
        .collect())
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

    #[test]
    fn conflict_policy_round_trips_through_rule_storage() {
        let state_dir = tempfile::tempdir().expect("state directory");
        let source = tempfile::tempdir().expect("source directory");
        let destination = tempfile::tempdir().expect("destination directory");
        let conflict_folder = tempfile::tempdir().expect("conflict directory");
        let db = AppStateDb::new(state_dir.path().to_path_buf()).expect("database");
        let rule = OrganizerRule::new_with_conflict_policy(
            0,
            source.path().to_path_buf(),
            destination.path().to_path_buf(),
            vec!["txt".to_string()],
            true,
            OrganizerConflictPolicy::MoveToConflictFolder(conflict_folder.path().to_path_buf()),
        )
        .expect("valid organizer rule");

        let id = db.save_organizer_rule(&rule).expect("save rule");
        let loaded = db
            .get_organizer_rules()
            .into_iter()
            .find(|candidate| candidate.id == id)
            .expect("load saved rule");

        assert_eq!(
            loaded.conflict_policy,
            OrganizerConflictPolicy::MoveToConflictFolder(conflict_folder.path().to_path_buf())
        );
    }

    #[test]
    fn saving_a_rule_with_an_extension_claimed_by_another_rule_is_blocked() {
        let state_dir = tempfile::tempdir().expect("state directory");
        let source_a = tempfile::tempdir().expect("source a");
        let source_b = tempfile::tempdir().expect("source b");
        let destination_a = tempfile::tempdir().expect("destination a");
        let destination_b = tempfile::tempdir().expect("destination b");
        let db = AppStateDb::new(state_dir.path().to_path_buf()).expect("database");
        let rule_a = OrganizerRule::new(
            0,
            source_a.path().to_path_buf(),
            destination_a.path().to_path_buf(),
            vec!["exe".to_string()],
            true,
        )
        .expect("rule a");
        db.save_organizer_rule(&rule_a).expect("save rule a");

        let rule_b = OrganizerRule::new(
            0,
            source_b.path().to_path_buf(),
            destination_b.path().to_path_buf(),
            vec!["exe".to_string()],
            true,
        )
        .expect("rule b");
        assert!(matches!(
            db.save_organizer_rule(&rule_b),
            Err(OrganizerRuleDbError::ExtensionConflict { ref extension, .. }) if extension == "exe"
        ));
        assert_eq!(db.get_organizer_rules().len(), 1);

        let rule_b_other_extension = OrganizerRule::new(
            0,
            source_b.path().to_path_buf(),
            destination_b.path().to_path_buf(),
            vec!["msi".to_string()],
            true,
        )
        .expect("rule b with another extension");
        db.save_organizer_rule(&rule_b_other_extension)
            .expect("non conflicting rule saves");
    }

    #[test]
    fn reenabling_a_disabled_rule_that_conflicts_is_blocked_on_save() {
        let state_dir = tempfile::tempdir().expect("state directory");
        let source = tempfile::tempdir().expect("source directory");
        let destination_a = tempfile::tempdir().expect("destination a");
        let destination_b = tempfile::tempdir().expect("destination b");
        let db = AppStateDb::new(state_dir.path().to_path_buf()).expect("database");
        let rule_a = OrganizerRule::new(
            0,
            source.path().to_path_buf(),
            destination_a.path().to_path_buf(),
            vec!["exe".to_string()],
            true,
        )
        .expect("rule a");
        db.save_organizer_rule(&rule_a).expect("save rule a");

        let rule_b = OrganizerRule::new(
            0,
            source.path().to_path_buf(),
            destination_b.path().to_path_buf(),
            vec!["exe".to_string()],
            false,
        )
        .expect("disabled rule b");
        let rule_b_id = db
            .save_organizer_rule(&rule_b)
            .expect("disabled rule does not conflict");

        let mut enabled_rule_b = OrganizerRule::new(
            rule_b_id,
            source.path().to_path_buf(),
            destination_b.path().to_path_buf(),
            vec!["exe".to_string()],
            true,
        )
        .expect("enabled rule b");
        assert!(matches!(
            db.save_organizer_rule(&enabled_rule_b),
            Err(OrganizerRuleDbError::ExtensionConflict { .. })
        ));

        enabled_rule_b.enabled = false;
        db.save_organizer_rule(&enabled_rule_b)
            .expect("staying disabled saves");
    }
}
