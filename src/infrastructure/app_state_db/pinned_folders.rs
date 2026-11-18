use super::{AppStateDb, AppStateWriteError};
use crate::domain::pinned_folder::PinnedFolder;
use crate::domain::quick_access_item::QuickAccessItem;
use rusqlite::{params, TransactionBehavior};

impl AppStateDb {
    /// Load all pinned folders ordered by position. [READER]
    pub fn get_all_pinned_folders(&self) -> Vec<PinnedFolder> {
        let mut results = Vec::new();
        let db = match self.reader.lock() {
            Ok(db) => db,
            Err(e) => {
                log::error!("[PINNED] Failed to acquire reader lock: {:?}", e);
                return results;
            }
        };
        let mut stmt = match db.prepare(
            "SELECT path, display_name, position FROM pinned_folders ORDER BY position ASC",
        ) {
            Ok(s) => s,
            Err(e) => {
                log::error!("[PINNED] Failed to prepare SELECT: {:?}", e);
                return results;
            }
        };
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        });
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (path, display_name, position) = row;
                results.push(PinnedFolder {
                    path,
                    display_name,
                    position,
                });
            }
        }
        log::info!("[PINNED] Loaded {} pinned folders", results.len());
        results
    }

    /// Save or update a pinned folder. [WRITER]
    pub fn save_pinned_folder(&self, path: &str, display_name: &str, position: i64) {
        if let Ok(db) = self.writer.lock() {
            match db.execute(
                "INSERT OR REPLACE INTO pinned_folders (path, display_name, position) VALUES (?, ?, ?)",
                params![path, display_name, position],
            ) {
                Ok(_) => log::info!("[PINNED] Saved {:?} at pos {}", path, position),
                Err(e) => log::error!("[PINNED] Failed to save {:?}: {:?}", path, e),
            }
        }
    }

    /// Remove a pinned folder. [WRITER]
    pub fn remove_pinned_folder(&self, path: &str) {
        if let Ok(db) = self.writer.lock() {
            let _ = db.execute("DELETE FROM pinned_folders WHERE path = ?", params![path]);
            log::info!("[PINNED] Removed {:?}", path);
        }
    }

    /// Reassign positions for the combined Quick Access sequence atomically. [WRITER]
    pub fn update_quick_access_positions(
        &self,
        ordered_items: &[QuickAccessItem],
    ) -> Result<(), AppStateWriteError> {
        let mut db = self
            .writer
            .lock()
            .map_err(|_| AppStateWriteError::WriterLockPoisoned)?;
        let transaction = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (position, item) in ordered_items.iter().enumerate() {
            match item {
                QuickAccessItem::PinnedFolder(path) => {
                    transaction.execute(
                        "UPDATE pinned_folders SET position = ? WHERE path = ?",
                        params![position as i64, path],
                    )?;
                }
                QuickAccessItem::FolderGroup(group_id) => {
                    transaction.execute(
                        "UPDATE folder_groups SET position = ? WHERE id = ?",
                        params![position as i64, group_id],
                    )?;
                }
            }
        }
        transaction.commit()?;
        Ok(())
    }
}
