use super::{AppStateDb, AppStateWriteError};
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashSet;

const GLOBAL_PAUSE_KEY: &str = "organizer_globally_paused";
const PAUSED_RULES_KEY: &str = "organizer_paused_rule_ids";

impl AppStateDb {
    pub(crate) fn organizer_globally_paused(&self) -> rusqlite::Result<bool> {
        match self
            .organizer_pause_preference(GLOBAL_PAUSE_KEY)?
            .as_deref()
        {
            None | Some("false") => Ok(false),
            Some("true") => Ok(true),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }

    pub(crate) fn set_organizer_globally_paused(
        &self,
        paused: bool,
    ) -> Result<(), AppStateWriteError> {
        self.set_preference(GLOBAL_PAUSE_KEY, if paused { "true" } else { "false" })
    }

    fn organizer_pause_preference(&self, key: &str) -> rusqlite::Result<Option<String>> {
        let db = self
            .reader
            .lock()
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        db.query_row(
            "SELECT value FROM user_preferences WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()
    }
}

/// Converts the removed individual pause state into each rule's existing toggle.
/// Consume the legacy preference in the same transaction so reopening the app
/// never disables a rule that the user has since explicitly enabled again.
pub(super) fn migrate_individual_pauses(conn: &Connection) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    let value: Option<String> = tx
        .query_row(
            "SELECT value FROM user_preferences WHERE key = ?1",
            [PAUSED_RULES_KEY],
            |row| row.get(0),
        )
        .optional()?;
    let Some(value) = value else {
        return Ok(());
    };
    let ids: HashSet<i64> = serde_json::from_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })?;
    for id in ids {
        tx.execute("UPDATE organizer_rules SET enabled = 0 WHERE id = ?1", [id])?;
    }
    tx.execute(
        "DELETE FROM user_preferences WHERE key = ?1",
        [PAUSED_RULES_KEY],
    )?;
    tx.commit()
}

#[cfg(test)]
#[path = "organizer_pause_tests.rs"]
mod tests;
