use super::AppStateDb;
use crate::domain::auto_tag_rule::AutoTagRule;
use rusqlite::{params, Connection};

#[cfg(test)]
mod crud_tests;

fn row_to_rule(row: &rusqlite::Row<'_>) -> rusqlite::Result<AutoTagRule> {
    Ok(AutoTagRule {
        id: row.get(0)?,
        folder_path: PathBuf::from(row.get::<_, String>(1)?),
        tag_id: row.get(2)?,
        enabled: row.get::<_, i64>(3)? != 0,
        include_subfolders: row.get::<_, i64>(4)? != 0,
        position: row.get(5)?,
    })
}

use std::path::PathBuf;

pub(super) fn create_auto_tag_rules_table(conn: &Connection) {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS auto_tag_rules (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            folder_path TEXT NOT NULL COLLATE NOCASE,
            tag_id INTEGER NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            include_subfolders INTEGER NOT NULL DEFAULT 1,
            position INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (tag_id) REFERENCES file_tags(id) ON DELETE CASCADE
        )",
        [],
    )
    .unwrap_or(0);

    // Migration: the subfolder option was added after the first release.
    if conn
        .prepare("SELECT include_subfolders FROM auto_tag_rules LIMIT 0")
        .is_err()
    {
        conn.execute(
            "ALTER TABLE auto_tag_rules ADD COLUMN include_subfolders INTEGER NOT NULL DEFAULT 1",
            [],
        )
        .unwrap_or(0);
    }

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_auto_tag_rules_folder
         ON auto_tag_rules(folder_path)",
        [],
    )
    .unwrap_or(0);
}

impl AppStateDb {
    /// Loads all auto-tag rules ordered by position. [READER]
    pub fn get_auto_tag_rules(&self) -> Vec<AutoTagRule> {
        let mut rules = Vec::new();
        let Ok(db) = self.reader.lock() else {
            log::error!("[AUTO-TAG] Failed to acquire reader lock for get_auto_tag_rules");
            return rules;
        };
        let Ok(mut stmt) = db.prepare(
            "SELECT id, folder_path, tag_id, enabled, include_subfolders, position
             FROM auto_tag_rules
             ORDER BY position ASC, id ASC",
        ) else {
            log::error!("[AUTO-TAG] Failed to prepare auto-tag rules SELECT");
            return rules;
        };
        let Ok(rows) = stmt.query_map([], row_to_rule) else {
            return rules;
        };
        for rule in rows.flatten() {
            rules.push(rule);
        }
        rules
    }

    /// Creates a rule and returns its id. [WRITER]
    pub fn create_auto_tag_rule(
        &self,
        folder_path: &std::path::Path,
        tag_id: i64,
        include_subfolders: bool,
    ) -> Option<i64> {
        let db = self.writer.lock().ok()?;
        let next_position = db
            .query_row(
                "SELECT COALESCE(MAX(position), -1) + 1 FROM auto_tag_rules",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0);
        match db.execute(
            "INSERT INTO auto_tag_rules (folder_path, tag_id, enabled, include_subfolders, position)
             SELECT ?1, ?2, 1, ?3, ?4 WHERE EXISTS (SELECT 1 FROM file_tags WHERE id = ?2)",
            params![
                folder_path.to_string_lossy(),
                tag_id,
                include_subfolders as i64,
                next_position
            ],
        ) {
            Ok(count) if count > 0 => Some(db.last_insert_rowid()),
            Ok(_) => None,
            Err(error) => {
                log::warn!("[AUTO-TAG] Failed to create rule: {error:?}");
                None
            }
        }
    }

    /// Updates a rule's folder, tag and subfolder option. Returns false when
    /// the rule is gone. [WRITER]
    pub fn update_auto_tag_rule(
        &self,
        id: i64,
        folder_path: &std::path::Path,
        tag_id: i64,
        include_subfolders: bool,
    ) -> bool {
        let Ok(db) = self.writer.lock() else {
            return false;
        };
        match db.execute(
            "UPDATE auto_tag_rules
             SET folder_path = ?1, tag_id = ?2, include_subfolders = ?3
              WHERE id = ?4 AND EXISTS (SELECT 1 FROM file_tags WHERE id = ?2)",
            params![
                folder_path.to_string_lossy(),
                tag_id,
                include_subfolders as i64,
                id
            ],
        ) {
            Ok(count) => count > 0,
            Err(error) => {
                log::warn!("[AUTO-TAG] Failed to update rule {id}: {error:?}");
                false
            }
        }
    }

    /// Enables or disables a rule. [WRITER]
    pub fn set_auto_tag_rule_enabled(&self, id: i64, enabled: bool) -> bool {
        let Ok(db) = self.writer.lock() else {
            return false;
        };
        matches!(
            db.execute(
                "UPDATE auto_tag_rules SET enabled = ?1 WHERE id = ?2",
                params![enabled as i64, id],
            ),
            Ok(count) if count > 0
        )
    }

    /// Toggles whether a rule covers subfolders or only direct children.
    /// [WRITER]
    pub fn set_auto_tag_rule_include_subfolders(&self, id: i64, include: bool) -> bool {
        let Ok(db) = self.writer.lock() else {
            return false;
        };
        matches!(
            db.execute(
                "UPDATE auto_tag_rules SET include_subfolders = ?1 WHERE id = ?2",
                params![include as i64, id],
            ),
            Ok(count) if count > 0
        )
    }

    /// Deletes a rule by id. [WRITER]
    pub fn delete_auto_tag_rule(&self, id: i64) -> bool {
        let Ok(db) = self.writer.lock() else {
            return false;
        };
        matches!(
            db.execute("DELETE FROM auto_tag_rules WHERE id = ?1", params![id]),
            Ok(count) if count > 0
        )
    }

    /// Renames the folder of the given rule (folder rename/move support).
    /// [WRITER]
    pub fn rename_auto_tag_rule_folder(&self, id: i64, new_folder: &std::path::Path) -> bool {
        let Ok(db) = self.writer.lock() else {
            return false;
        };
        matches!(
            db.execute(
                "UPDATE auto_tag_rules SET folder_path = ?1 WHERE id = ?2",
                params![new_folder.to_string_lossy(), id],
            ),
            Ok(count) if count > 0
        )
    }
}
