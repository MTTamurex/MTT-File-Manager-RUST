use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct IndexedFile {
    pub name: String,
    pub size: u64,
    pub modified: u64,
    pub is_dir: bool,
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
                scan_duration_ms INTEGER NOT NULL
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

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn get_directory(&self, dir_path: &Path) -> Option<(DirectoryMeta, Vec<IndexedFile>)> {
        let conn = self.conn.lock().ok()?;
        let dir_str = dir_path.to_string_lossy();

        let meta: DirectoryMeta = conn
            .query_row(
                "SELECT file_count, total_size, last_scan_time, scan_duration_ms
                 FROM directory_index WHERE dir_path = ?",
                [&dir_str],
                |row| {
                    Ok(DirectoryMeta {
                        file_count: row.get::<_, i64>(0)? as usize,
                        total_size: row.get::<_, i64>(1)? as u64,
                        last_scan: row.get::<_, i64>(2)? as u64,
                        scan_duration_ms: row.get::<_, i64>(3)? as u64,
                    })
                },
            )
            .ok()?;

        let mut stmt = conn
            .prepare_cached(
                "SELECT file_name, file_size, modified_time, is_dir
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
                })
            })
            .ok()?
            .filter_map(|r| r.ok())
            .collect();

        Some((meta, files))
    }

    /// Non-blocking variant for UI-thread callers.
    /// Returns `None` immediately if the connection lock is held by a writer.
    pub fn try_get_directory(&self, dir_path: &Path) -> Option<(DirectoryMeta, Vec<IndexedFile>)> {
        let conn = match self.conn.try_lock() {
            Ok(c) => c,
            Err(_) => return None, // Writer busy — treat as cache miss
        };
        let dir_str = dir_path.to_string_lossy();

        let meta: DirectoryMeta = conn
            .query_row(
                "SELECT file_count, total_size, last_scan_time, scan_duration_ms
                 FROM directory_index WHERE dir_path = ?",
                [&dir_str],
                |row| {
                    Ok(DirectoryMeta {
                        file_count: row.get::<_, i64>(0)? as usize,
                        total_size: row.get::<_, i64>(1)? as u64,
                        last_scan: row.get::<_, i64>(2)? as u64,
                        scan_duration_ms: row.get::<_, i64>(3)? as u64,
                    })
                },
            )
            .ok()?;

        let mut stmt = conn
            .prepare_cached(
                "SELECT file_name, file_size, modified_time, is_dir
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
                })
            })
            .ok()?
            .filter_map(|r| r.ok())
            .collect();

        Some((meta, files))
    }

    pub fn put_directory(
        &self,
        dir_path: &Path,
        files: &[IndexedFile],
        scan_duration_ms: u64,
    ) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().map_err(|_| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                Some("Lock poisoned".to_string()),
            )
        })?;

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
                "INSERT INTO file_index (dir_path, file_name, file_size, modified_time, is_dir)
                 VALUES (?, ?, ?, ?, ?)",
            )?;

            for file in files {
                stmt.execute(params![
                    &dir_str,
                    &file.name,
                    file.size as i64,
                    file.modified as i64,
                    if file.is_dir { 1 } else { 0 },
                ])?;
            }
        }

        tx.execute(
            "INSERT OR REPLACE INTO directory_index
             (dir_path, file_count, total_size, last_scan_time, scan_duration_ms)
             VALUES (?, ?, ?, ?, ?)",
            params![
                &dir_str,
                files.len() as i64,
                total_size as i64,
                now as i64,
                scan_duration_ms as i64,
            ],
        )?;

        tx.commit()?;
        Ok(())
    }

    pub fn invalidate(&self, dir_path: &Path) -> rusqlite::Result<()> {
        let conn = self.conn.lock().map_err(|_| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                Some("Lock poisoned".to_string()),
            )
        })?;

        let dir_str = dir_path.to_string_lossy();

        conn.execute("DELETE FROM file_index WHERE dir_path = ?", [&dir_str])?;
        conn.execute("DELETE FROM directory_index WHERE dir_path = ?", [&dir_str])?;

        Ok(())
    }

    pub fn invalidate_recursive(&self, parent: &Path) -> rusqlite::Result<()> {
        let conn = self.conn.lock().map_err(|_| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                Some("Lock poisoned".to_string()),
            )
        })?;

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
        let conn = self.conn.lock().ok()?;

        let dir_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM directory_index", [], |row| row.get(0))
            .ok()?;

        let file_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM file_index", [], |row| row.get(0))
            .ok()?;

        Some((dir_count as usize, file_count as usize))
    }
}
