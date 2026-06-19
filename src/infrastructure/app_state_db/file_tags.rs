use super::AppStateDb;
use crate::domain::file_tag::{FileTag, TagColor};
use rusqlite::{params, Connection, OptionalExtension};
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};

fn normalize_path_text(path: &str) -> String {
    path.replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn path_text_is_same_or_descendant(candidate: &str, root: &str) -> bool {
    let candidate = normalize_path_text(candidate);
    let root = normalize_path_text(root);
    candidate == root
        || candidate
            .strip_prefix(&root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

fn remap_path_text(candidate: &str, old_root: &str, new_root: &str) -> Option<String> {
    if !path_text_is_same_or_descendant(candidate, old_root) {
        return None;
    }

    let old_root = old_root.trim_end_matches(['\\', '/']);
    if candidate.len() <= old_root.len() {
        return Some(new_root.to_string());
    }

    let suffix = &candidate[old_root.len()..];
    Some(format!(
        "{}{}",
        new_root.trim_end_matches(['\\', '/']),
        suffix
    ))
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

    for (position, color) in TagColor::default_palette().into_iter().enumerate() {
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
                results.entry(PathBuf::from(path)).or_default().push(tag_id);
            }
        }

        results
    }

    /// Assign a tag to a path. [WRITER]
    pub fn assign_tag(&self, path: &Path, tag_id: i64) -> bool {
        let Some(path) = path.to_str() else {
            return false;
        };
        if let Ok(db) = self.writer.lock() {
            db.execute(
                "INSERT OR IGNORE INTO file_tag_assignments (file_path, tag_id) VALUES (?1, ?2)",
                params![path, tag_id],
            )
            .is_ok()
        } else {
            false
        }
    }

    /// Remove one tag assignment from a path. [WRITER]
    pub fn unassign_tag(&self, path: &Path, tag_id: i64) -> bool {
        let Some(path) = path.to_str() else {
            return false;
        };
        if let Ok(db) = self.writer.lock() {
            db.execute(
                "DELETE FROM file_tag_assignments WHERE file_path = ?1 AND tag_id = ?2",
                params![path, tag_id],
            )
            .is_ok()
        } else {
            false
        }
    }

    /// Delete all assignments for the given paths. [WRITER]
    pub fn clear_tag_assignments_for_paths(&self, paths: &[PathBuf]) -> usize {
        if paths.is_empty() {
            return 0;
        }

        let roots: Vec<String> = paths
            .iter()
            .filter_map(|path| path.to_str().map(ToOwned::to_owned))
            .collect();
        if roots.is_empty() {
            return 0;
        }

        let mut db = match self.writer.lock() {
            Ok(db) => db,
            Err(_) => return 0,
        };

        let paths_to_delete: FxHashSet<String> = db
            .prepare("SELECT file_path FROM file_tag_assignments")
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get::<_, String>(0))
                    .map(|rows| rows.flatten().collect::<Vec<_>>())
            })
            .unwrap_or_default()
            .into_iter()
            .filter(|file_path| {
                roots
                    .iter()
                    .any(|root| path_text_is_same_or_descendant(file_path, root))
            })
            .collect();
        if paths_to_delete.is_empty() {
            return 0;
        }

        let mut removed = 0;
        let tx = match db.transaction() {
            Ok(tx) => tx,
            Err(_) => return 0,
        };
        for path in paths_to_delete {
            removed += tx
                .execute(
                    "DELETE FROM file_tag_assignments WHERE file_path = ?1",
                    params![path],
                )
                .unwrap_or(0);
        }
        let _ = tx.commit();
        removed
    }

    /// Move assignments from one path to another. [WRITER]
    pub fn move_tag_assignments(&self, old_path: &Path, new_path: &Path) -> bool {
        let (Some(old_path), Some(new_path)) = (old_path.to_str(), new_path.to_str()) else {
            return false;
        };
        if old_path == new_path {
            return true;
        }

        let mut db = match self.writer.lock() {
            Ok(db) => db,
            Err(_) => return false,
        };

        let remapped_rows: Vec<(String, String, i64)> = db
            .prepare("SELECT file_path, tag_id FROM file_tag_assignments")
            .and_then(|mut stmt| {
                stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .map(|rows| rows.flatten().collect::<Vec<(String, i64)>>())
            })
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(file_path, tag_id)| {
                remap_path_text(&file_path, old_path, new_path)
                    .map(|new_file_path| (file_path, new_file_path, tag_id))
            })
            .collect();
        if remapped_rows.is_empty() {
            return true;
        }

        let tx = match db.transaction() {
            Ok(tx) => tx,
            Err(_) => return false,
        };
        for (old_file_path, new_file_path, tag_id) in remapped_rows {
            let _ = tx.execute(
                "INSERT OR IGNORE INTO file_tag_assignments (file_path, tag_id) VALUES (?1, ?2)",
                params![new_file_path, tag_id],
            );
            let _ = tx.execute(
                "DELETE FROM file_tag_assignments WHERE file_path = ?1 AND tag_id = ?2",
                params![old_file_path, tag_id],
            );
        }
        tx.commit().is_ok()
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
