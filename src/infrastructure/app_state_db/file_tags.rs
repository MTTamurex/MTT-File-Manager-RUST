use super::AppStateDb;
use crate::domain::file_tag::{FileTag, TagColor};
use rusqlite::{params, Connection, OptionalExtension};
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};

fn normalize_path_text(path: &str) -> String {
    crate::domain::file_tag::normalize_tag_path_text(path)
}

fn path_match_key(path: &str) -> String {
    normalize_path_text(path).to_lowercase()
}

fn storage_path_text(path: &Path) -> Option<String> {
    path.to_str().map(normalize_path_text)
}

fn storage_path_texts(paths: &[PathBuf]) -> Vec<String> {
    let mut seen = FxHashSet::default();
    paths
        .iter()
        .filter_map(|path| storage_path_text(path))
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

pub(super) fn seed_default_file_tags(conn: &Connection) {
    let existing_count = conn
        .query_row("SELECT COUNT(*) FROM file_tags", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap_or(0);
    if existing_count > 0 {
        return;
    }

    for (position, color) in TagColor::default_palette().into_iter().take(5).enumerate() {
        let name = match color {
            TagColor::Red => "Red",
            TagColor::Orange => "Orange",
            TagColor::Yellow => "Yellow",
            TagColor::Green => "Green",
            TagColor::Blue => "Blue",
            TagColor::Purple => "Purple",
            TagColor::Gray => "Gray",
        };
        let _ = conn.execute(
            "INSERT OR IGNORE INTO file_tags (name, color, position) VALUES (?1, ?2, ?3)",
            params![name, color.as_db_str(), position as i64],
        );
    }
}

impl AppStateDb {
    /// Load all tag definitions ordered by display position. [READER]
    pub fn get_all_tags(&self) -> Vec<FileTag> {
        let mut results = Vec::new();
        let db = match self.reader.lock() {
            Ok(db) => db,
            Err(e) => {
                log::error!("[TAGS] Failed to acquire reader lock: {:?}", e);
                return results;
            }
        };

        let mut stmt = match db.prepare(
            "SELECT id, name, color, position FROM file_tags ORDER BY position ASC, name ASC",
        ) {
            Ok(stmt) => stmt,
            Err(e) => {
                log::error!("[TAGS] Failed to prepare tag SELECT: {:?}", e);
                return results;
            }
        };

        let rows = stmt.query_map([], |row| {
            let color_raw: String = row.get(2)?;
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                color_raw,
                row.get::<_, i64>(3)?,
            ))
        });

        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (id, name, color_raw, position) = row;
                if let Some(color) = TagColor::from_db_str(&color_raw) {
                    results.push(FileTag {
                        id,
                        name,
                        color,
                        position,
                    });
                } else {
                    log::warn!(
                        "[TAGS] Ignoring tag {} with invalid color {}",
                        id,
                        color_raw
                    );
                }
            }
        }

        results
    }

    /// Ensures default color tags exist. [WRITER]
    pub fn ensure_default_tags(&self) {
        if let Ok(db) = self.writer.lock() {
            seed_default_file_tags(&db);
        }
    }

    /// Create a custom tag and return its new id. [WRITER]
    pub fn create_tag(&self, name: &str, color: TagColor) -> Option<i64> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }

        let db = match self.writer.lock() {
            Ok(db) => db,
            Err(e) => {
                log::error!(
                    "[TAGS] Failed to acquire writer lock for create_tag: {:?}",
                    e
                );
                return None;
            }
        };

        let next_position = db
            .query_row(
                "SELECT COALESCE(MAX(position), -1) + 1 FROM file_tags",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0);

        match db.execute(
            "INSERT INTO file_tags (name, color, position) VALUES (?1, ?2, ?3)",
            params![name, color.as_db_str(), next_position],
        ) {
            Ok(_) => Some(db.last_insert_rowid()),
            Err(e) => {
                log::warn!("[TAGS] Failed to create tag {:?}: {:?}", name, e);
                None
            }
        }
    }

    /// Rename an existing tag. [WRITER]
    pub fn rename_tag(&self, id: i64, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        if let Ok(db) = self.writer.lock() {
            match db.execute(
                "UPDATE file_tags SET name = ?1 WHERE id = ?2",
                params![name, id],
            ) {
                Ok(count) => count > 0,
                Err(e) => {
                    log::warn!("[TAGS] Failed to rename tag {}: {:?}", id, e);
                    false
                }
            }
        } else {
            false
        }
    }

    /// Update tag color. [WRITER]
    pub fn update_tag_color(&self, id: i64, color: TagColor) -> bool {
        if let Ok(db) = self.writer.lock() {
            match db.execute(
                "UPDATE file_tags SET color = ?1 WHERE id = ?2",
                params![color.as_db_str(), id],
            ) {
                Ok(count) => count > 0,
                Err(e) => {
                    log::warn!("[TAGS] Failed to recolor tag {}: {:?}", id, e);
                    false
                }
            }
        } else {
            false
        }
    }

    /// Delete a tag and its assignments. [WRITER]
    pub fn delete_tag(&self, id: i64) -> bool {
        let mut db = match self.writer.lock() {
            Ok(db) => db,
            Err(e) => {
                log::error!(
                    "[TAGS] Failed to acquire writer lock for delete_tag: {:?}",
                    e
                );
                return false;
            }
        };

        let tx = match db.transaction() {
            Ok(tx) => tx,
            Err(e) => {
                log::warn!("[TAGS] Failed to start delete transaction: {:?}", e);
                return false;
            }
        };
        let _ = tx.execute(
            "DELETE FROM file_tag_assignments WHERE tag_id = ?1",
            params![id],
        );
        let deleted = tx
            .execute("DELETE FROM file_tags WHERE id = ?1", params![id])
            .unwrap_or(0);
        if tx.commit().is_err() {
            return false;
        }
        deleted > 0
    }

    /// Load all path -> tag assignments. [READER]
    pub fn get_all_tag_assignments(&self) -> FxHashMap<PathBuf, Vec<i64>> {
        let mut results: FxHashMap<PathBuf, Vec<i64>> = FxHashMap::default();
        let db = match self.reader.lock() {
            Ok(db) => db,
            Err(e) => {
                log::error!("[TAGS] Failed to acquire reader lock: {:?}", e);
                return results;
            }
        };

        let mut stmt = match db.prepare(
            "SELECT file_path, tag_id FROM file_tag_assignments ORDER BY file_path ASC, tag_id ASC",
        ) {
            Ok(stmt) => stmt,
            Err(e) => {
                log::error!("[TAGS] Failed to prepare assignments SELECT: {:?}", e);
                return results;
            }
        };

        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        }) {
            for row in rows.flatten() {
                let (path, tag_id) = row;
                results
                    .entry(PathBuf::from(normalize_path_text(&path)))
                    .or_default()
                    .push(tag_id);
            }
        }

        results
    }

    /// Load paths assigned to a specific tag, using the `idx_file_tag_assignments_tag`
    /// index for an efficient filtered lookup instead of a full table scan. [READER]
    pub fn get_tag_assignment_paths(&self, tag_id: i64) -> Vec<PathBuf> {
        let db = match self.reader.lock() {
            Ok(db) => db,
            Err(e) => {
                log::error!(
                    "[TAGS] Failed to acquire reader lock for get_tag_assignment_paths: {:?}",
                    e
                );
                return Vec::new();
            }
        };
        let mut stmt = match db.prepare(
            "SELECT file_path FROM file_tag_assignments \
             WHERE tag_id = ?1 ORDER BY file_path ASC",
        ) {
            Ok(stmt) => stmt,
            Err(e) => {
                log::error!("[TAGS] Failed to prepare tag paths SELECT: {:?}", e);
                return Vec::new();
            }
        };
        stmt.query_map(params![tag_id], |row| {
            row.get::<_, String>(0)
                .map(|s| PathBuf::from(normalize_path_text(&s)))
        })
        .ok()
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    /// Assign a tag to many paths in a single transaction. [WRITER]
    pub fn assign_tag_batch(&self, paths: &[PathBuf], tag_id: i64) -> bool {
        let paths = storage_path_texts(paths);
        if paths.is_empty() {
            return false;
        }

        let mut db = match self.writer.lock() {
            Ok(db) => db,
            Err(e) => {
                log::error!(
                    "[TAGS] Failed to acquire writer lock for assign_tag_batch: {:?}",
                    e
                );
                return false;
            }
        };

        let result = (|| -> rusqlite::Result<()> {
            let tx = db.transaction()?;
            {
                let mut stmt = tx.prepare(
                    "INSERT OR IGNORE INTO file_tag_assignments (file_path, tag_id) VALUES (?1, ?2)",
                )?;
                for path in &paths {
                    stmt.execute(params![path, tag_id])?;
                }
            }
            tx.commit()
        })();

        if let Err(error) = result {
            log::warn!("[TAGS] Failed to assign tag batch: {:?}", error);
            return false;
        }
        true
    }

    /// Remove one tag assignment from many paths in a single transaction. [WRITER]
    pub fn unassign_tag_batch(&self, paths: &[PathBuf], tag_id: i64) -> bool {
        let paths = storage_path_texts(paths);
        if paths.is_empty() {
            return false;
        }

        let mut db = match self.writer.lock() {
            Ok(db) => db,
            Err(e) => {
                log::error!(
                    "[TAGS] Failed to acquire writer lock for unassign_tag_batch: {:?}",
                    e
                );
                return false;
            }
        };

        let result = (|| -> rusqlite::Result<()> {
            let tx = db.transaction()?;
            {
                let mut stmt = tx.prepare(
                    "DELETE FROM file_tag_assignments WHERE file_path = ?1 AND tag_id = ?2",
                )?;
                for path in &paths {
                    stmt.execute(params![path, tag_id])?;
                }
            }
            tx.commit()
        })();

        if let Err(error) = result {
            log::warn!("[TAGS] Failed to unassign tag batch: {:?}", error);
            return false;
        }
        true
    }

    /// Delete all assignments for exact assignment paths. [WRITER]
    pub fn clear_tag_assignments_for_paths(&self, paths: &[PathBuf]) -> Option<usize> {
        if paths.is_empty() {
            return Some(0);
        }

        let paths_to_delete = storage_path_texts(paths);
        if paths_to_delete.is_empty() {
            return Some(0);
        }

        let mut db = match self.writer.lock() {
            Ok(db) => db,
            Err(e) => {
                log::error!(
                    "[TAGS] Failed to acquire writer lock for clear_tag_assignments_for_paths: {:?}",
                    e
                );
                return None;
            }
        };

        let result = (|| -> rusqlite::Result<usize> {
            let tx = db.transaction()?;
            let mut removed = 0;
            const BATCH_SIZE: usize = 500;
            for chunk in paths_to_delete.chunks(BATCH_SIZE) {
                let placeholders = std::iter::repeat_n("?", chunk.len())
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "DELETE FROM file_tag_assignments WHERE file_path IN ({})",
                    placeholders
                );
                removed += tx.execute(&sql, rusqlite::params_from_iter(chunk.iter()))?;
            }
            tx.commit()?;
            Ok(removed)
        })();

        match result {
            Ok(removed) => Some(removed),
            Err(error) => {
                log::warn!("[TAGS] Failed to clear tag assignments: {:?}", error);
                None
            }
        }
    }

    /// Move exact assignment rows in a single transaction. [WRITER]
    pub fn move_tag_assignments(&self, moved_assignments: &[(PathBuf, PathBuf, i64)]) -> bool {
        if moved_assignments.is_empty() {
            return true;
        }

        let remapped_rows: Vec<(String, String, i64)> = moved_assignments
            .iter()
            .filter_map(|(old_path, new_path, tag_id)| {
                let old_path = storage_path_text(old_path)?;
                let new_path = storage_path_text(new_path)?;
                (old_path != new_path).then_some((old_path, new_path, *tag_id))
            })
            .collect();
        if remapped_rows.is_empty() {
            return true;
        }

        let mut db = match self.writer.lock() {
            Ok(db) => db,
            Err(e) => {
                log::error!(
                    "[TAGS] Failed to acquire writer lock for move_tag_assignments: {:?}",
                    e
                );
                return false;
            }
        };

        let result = (|| -> rusqlite::Result<()> {
            let tx = db.transaction()?;
            {
                let mut insert_stmt = tx.prepare(
                    "INSERT OR IGNORE INTO file_tag_assignments (file_path, tag_id) VALUES (?1, ?2)",
                )?;
                let mut delete_stmt = tx.prepare(
                    "DELETE FROM file_tag_assignments WHERE file_path = ?1 AND tag_id = ?2",
                )?;
                let mut update_stmt = tx.prepare(
                    "UPDATE file_tag_assignments SET file_path = ?1 WHERE file_path = ?2 AND tag_id = ?3",
                )?;

                for (old_file_path, new_file_path, tag_id) in &remapped_rows {
                    if path_match_key(old_file_path) == path_match_key(new_file_path) {
                        update_stmt.execute(params![new_file_path, old_file_path, tag_id])?;
                    } else {
                        insert_stmt.execute(params![new_file_path, tag_id])?;
                        delete_stmt.execute(params![old_file_path, tag_id])?;
                    }
                }
            }
            tx.commit()
        })();

        if let Err(error) = result {
            log::warn!("[TAGS] Failed to move tag assignments: {:?}", error);
            return false;
        }
        true
    }

    /// Counts assignments per tag. [READER]
    pub fn get_tag_counts(&self) -> FxHashMap<i64, usize> {
        let mut results = FxHashMap::default();
        let db = match self.reader.lock() {
            Ok(db) => db,
            Err(_) => return results,
        };
        if let Ok(mut stmt) = db.prepare(
            "SELECT tag_id, COUNT(*) FROM file_tag_assignments GROUP BY tag_id ORDER BY tag_id",
        ) {
            if let Ok(rows) =
                stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
            {
                for row in rows.flatten() {
                    results.insert(row.0, row.1.max(0) as usize);
                }
            }
        }
        results
    }

    pub fn get_tag_assignment_count(&self, tag_id: i64) -> usize {
        if let Ok(db) = self.reader.lock() {
            db.query_row(
                "SELECT COUNT(*) FROM file_tag_assignments WHERE tag_id = ?1",
                params![tag_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .ok()
            .flatten()
            .unwrap_or(0)
            .max(0) as usize
        } else {
            0
        }
    }
}
