use parking_lot::Mutex;
use rusqlite::{params, Connection};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const DIRECTORY_INDEX_FORMAT_VERSION: i64 = 1;

#[derive(Debug, Clone)]
pub struct IndexedFile {
    pub name: String,
    pub size: u64,
    pub modified: u64,
    pub is_dir: bool,
    pub created: u64,
}

#[derive(Debug, Clone)]
pub struct DirectoryMeta {
    pub file_count: usize,
    pub total_size: u64,
    pub last_scan: u64,
    pub scan_duration_ms: u64,
}

pub struct DirectoryIndex {
    conn: Mutex<Connection>,
}

impl DirectoryIndex {
    pub fn open(db_path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS directory_index (
                dir_path TEXT PRIMARY KEY,
                file_count INTEGER NOT NULL,
                total_size INTEGER NOT NULL,
                last_scan_time INTEGER NOT NULL,
                scan_duration_ms INTEGER NOT NULL,
                format_version INTEGER NOT NULL DEFAULT 1
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS file_index (
                id INTEGER PRIMARY KEY,
                dir_path TEXT NOT NULL,
                file_name TEXT NOT NULL,
                file_size INTEGER NOT NULL,
                modified_time INTEGER NOT NULL,
                is_dir INTEGER NOT NULL,
                UNIQUE(dir_path, file_name)
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_file_index_dir ON file_index(dir_path)",
            [],
        )?;

        // Migration: add created_time column if it doesn't exist (v2 schema).
        let has_created: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('file_index') WHERE name = 'created_time'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_created {
            let _ = conn.execute(
                "ALTER TABLE file_index ADD COLUMN created_time INTEGER NOT NULL DEFAULT 0",
                [],
            );
        }

        // Older indexes omitted dot-prefixed Windows entries. Keep their rows,
        // but mark them as legacy so each directory is rebuilt when revisited.
        let has_format_version: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('directory_index') WHERE name = 'format_version'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_format_version {
            conn.execute(
                "ALTER TABLE directory_index ADD COLUMN format_version INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn get_directory(&self, dir_path: &Path) -> Option<(DirectoryMeta, Vec<IndexedFile>)> {
        let conn = self.conn.lock();
        let dir_str = dir_path.to_string_lossy();

        let (meta, format_version): (DirectoryMeta, i64) = conn
            .query_row(
                "SELECT file_count, total_size, last_scan_time, scan_duration_ms, format_version
                 FROM directory_index WHERE dir_path = ?",
                [&dir_str],
                |row| {
                    Ok((
                        DirectoryMeta {
                            file_count: row.get::<_, i64>(0)? as usize,
                            total_size: row.get::<_, i64>(1)? as u64,
                            last_scan: row.get::<_, i64>(2)? as u64,
                            scan_duration_ms: row.get::<_, i64>(3)? as u64,
                        },
                        row.get(4)?,
                    ))
                },
            )
            .ok()?;

        if format_version != DIRECTORY_INDEX_FORMAT_VERSION {
            log::debug!(
                "[DirectoryIndex] Ignoring legacy cache format {} for {}",
                format_version,
                dir_str
            );
            return None;
        }

        let mut stmt = conn
            .prepare_cached(
                "SELECT file_name, file_size, modified_time, is_dir, created_time
                 FROM file_index WHERE dir_path = ?",
            )
            .ok()?;

        let files: Vec<IndexedFile> = stmt
            .query_map([&dir_str], |row| {
                Ok(IndexedFile {
                    name: row.get(0)?,
                    size: row.get::<_, i64>(1)? as u64,
                    modified: row.get::<_, i64>(2)? as u64,
                    is_dir: row.get::<_, i64>(3)? != 0,
                    created: row.get::<_, i64>(4).unwrap_or(0) as u64,
                })
            })
            .ok()?
            .filter_map(|r| r.ok())
            .collect();

        if has_legacy_created_metadata(&files) {
            log::debug!(
                "[DirectoryIndex] Ignoring legacy cache without created_time for {}",
                dir_str
            );
            return None;
        }

        Some((meta, files))
    }

    /// Non-blocking variant for UI-thread callers.
    /// Returns `None` immediately if the connection lock is held by a writer.
    pub fn try_get_directory(&self, dir_path: &Path) -> Option<(DirectoryMeta, Vec<IndexedFile>)> {
        let conn = self.conn.try_lock()?;
        let dir_str = dir_path.to_string_lossy();

        let (meta, format_version): (DirectoryMeta, i64) = conn
            .query_row(
                "SELECT file_count, total_size, last_scan_time, scan_duration_ms, format_version
                 FROM directory_index WHERE dir_path = ?",
                [&dir_str],
                |row| {
                    Ok((
                        DirectoryMeta {
                            file_count: row.get::<_, i64>(0)? as usize,
                            total_size: row.get::<_, i64>(1)? as u64,
                            last_scan: row.get::<_, i64>(2)? as u64,
                            scan_duration_ms: row.get::<_, i64>(3)? as u64,
                        },
                        row.get(4)?,
                    ))
                },
            )
            .ok()?;

        if format_version != DIRECTORY_INDEX_FORMAT_VERSION {
            log::debug!(
                "[DirectoryIndex] Ignoring legacy cache format {} for {}",
                format_version,
                dir_str
            );
            return None;
        }

        let mut stmt = conn
            .prepare_cached(
                "SELECT file_name, file_size, modified_time, is_dir, created_time
                 FROM file_index WHERE dir_path = ?",
            )
            .ok()?;

        let files: Vec<IndexedFile> = stmt
            .query_map([&dir_str], |row| {
                Ok(IndexedFile {
                    name: row.get(0)?,
                    size: row.get::<_, i64>(1)? as u64,
                    modified: row.get::<_, i64>(2)? as u64,
                    is_dir: row.get::<_, i64>(3)? != 0,
                    created: row.get::<_, i64>(4).unwrap_or(0) as u64,
                })
            })
            .ok()?
            .filter_map(|r| r.ok())
            .collect();

        if has_legacy_created_metadata(&files) {
            log::debug!(
                "[DirectoryIndex] Ignoring legacy cache without created_time for {}",
                dir_str
            );
            return None;
        }

        Some((meta, files))
    }

    pub fn put_directory(
        &self,
        dir_path: &Path,
        files: &[IndexedFile],
        scan_duration_ms: u64,
    ) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock();

        let dir_str_cow = dir_path.to_string_lossy();
        let dir_str: &str = &dir_str_cow;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let total_size: u64 = files.iter().map(|f| f.size).sum();

        let tx = conn.transaction()?;

        tx.execute("DELETE FROM file_index WHERE dir_path = ?", [&dir_str])?;

        {
            let mut stmt = tx.prepare(
                "INSERT INTO file_index (dir_path, file_name, file_size, modified_time, is_dir, created_time)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )?;

            for file in files {
                stmt.execute(params![
                    &dir_str,
                    &file.name,
                    file.size as i64,
                    file.modified as i64,
                    if file.is_dir { 1 } else { 0 },
                    file.created as i64,
                ])?;
            }
        }

        tx.execute(
            "INSERT OR REPLACE INTO directory_index
             (dir_path, file_count, total_size, last_scan_time, scan_duration_ms, format_version)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![
                &dir_str,
                files.len() as i64,
                total_size as i64,
                now as i64,
                scan_duration_ms as i64,
                DIRECTORY_INDEX_FORMAT_VERSION,
            ],
        )?;

        tx.commit()?;
        Ok(())
    }

    pub fn invalidate(&self, dir_path: &Path) -> rusqlite::Result<()> {
        let conn = self.conn.lock();

        let dir_str = dir_path.to_string_lossy();

        conn.execute("DELETE FROM file_index WHERE dir_path = ?", [&dir_str])?;
        conn.execute("DELETE FROM directory_index WHERE dir_path = ?", [&dir_str])?;

        Ok(())
    }

    pub fn invalidate_recursive(&self, parent: &Path) -> rusqlite::Result<()> {
        let conn = self.conn.lock();

        let parent_str = format!("{}%", parent.to_string_lossy());

        conn.execute(
            "DELETE FROM file_index WHERE dir_path LIKE ?",
            [&parent_str],
        )?;
        conn.execute(
            "DELETE FROM directory_index WHERE dir_path LIKE ?",
            [&parent_str],
        )?;

        Ok(())
    }

    pub fn stats(&self) -> Option<(usize, usize)> {
        let conn = self.conn.lock();

        let dir_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM directory_index", [], |row| row.get(0))
            .ok()?;

        let file_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM file_index", [], |row| row.get(0))
            .ok()?;

        Some((dir_count as usize, file_count as usize))
    }
}

fn has_legacy_created_metadata(files: &[IndexedFile]) -> bool {
    files.iter().any(|file| file.created == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn indexed_file(created: u64) -> IndexedFile {
        IndexedFile {
            name: "file.txt".to_string(),
            size: 10,
            modified: 1_700_000_000,
            is_dir: false,
            created,
        }
    }

    #[test]
    fn get_directory_rejects_legacy_rows_without_created_time() {
        let temp = tempfile::tempdir().unwrap();
        let index = DirectoryIndex::open(&temp.path().join("directory_cache.db")).unwrap();
        let dir = temp.path().join("folder");

        index.put_directory(&dir, &[indexed_file(0)], 1).unwrap();

        assert!(index.get_directory(&dir).is_none());
        assert!(index.try_get_directory(&dir).is_none());
    }

    #[test]
    fn get_directory_returns_rows_with_created_time() {
        let temp = tempfile::tempdir().unwrap();
        let index = DirectoryIndex::open(&temp.path().join("directory_cache.db")).unwrap();
        let dir = temp.path().join("folder");

        index
            .put_directory(&dir, &[indexed_file(1_600_000_000)], 1)
            .unwrap();

        let (_, files) = index.get_directory(&dir).unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].created, 1_600_000_000);
    }

    #[test]
    fn get_directory_rejects_rows_from_previous_format() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("directory_cache.db");
        let dir = temp.path().join("folder");
        let dir_str = dir.to_string_lossy().into_owned();

        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE directory_index (
                    dir_path TEXT PRIMARY KEY,
                    file_count INTEGER NOT NULL,
                    total_size INTEGER NOT NULL,
                    last_scan_time INTEGER NOT NULL,
                    scan_duration_ms INTEGER NOT NULL
                );
                CREATE TABLE file_index (
                    id INTEGER PRIMARY KEY,
                    dir_path TEXT NOT NULL,
                    file_name TEXT NOT NULL,
                    file_size INTEGER NOT NULL,
                    modified_time INTEGER NOT NULL,
                    is_dir INTEGER NOT NULL,
                    created_time INTEGER NOT NULL DEFAULT 0,
                    UNIQUE(dir_path, file_name)
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO directory_index
                 (dir_path, file_count, total_size, last_scan_time, scan_duration_ms)
                 VALUES (?, 1, 10, 1700000000, 1)",
                [&dir_str],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO file_index
                 (dir_path, file_name, file_size, modified_time, is_dir, created_time)
                 VALUES (?, 'file.txt', 10, 1700000000, 0, 1600000000)",
                [&dir_str],
            )
            .unwrap();
        }

        let index = DirectoryIndex::open(&db_path).unwrap();

        assert!(index.get_directory(&dir).is_none());
        assert!(index.try_get_directory(&dir).is_none());
    }
}
