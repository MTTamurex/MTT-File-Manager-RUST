mod fts;
mod sync;

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use parking_lot::Mutex;

use rusqlite::{params, Connection};

pub use fts::FtsSearcher;

/// Persisted volume state for fast restart.
pub struct PersistedVolumeState {
    pub drive_letter: char,
    pub journal_id: u64,
    pub last_usn: i64,
    pub files_indexed: u64,
}

/// SQLite-based persistence for the file index.
/// Wrapped in Mutex because rusqlite::Connection is not Sync.
pub struct IndexDb {
    conn: Mutex<Connection>,
}

/// Get the database file path.
pub fn get_db_path() -> Result<PathBuf, String> {
    let base = std::env::var("PROGRAMDATA").unwrap_or_else(|_| r"C:\ProgramData".to_string());
    let dir = Path::new(&base).join("MTT-File-Manager");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create ProgramData directory {:?}: {}", dir, e))?;

    let dir_str = dir.to_string_lossy().to_string();
    let acl_commands: &[&[&str]] = &[
        &[&dir_str, "/inheritance:r"],
        &[&dir_str, "/grant:r", "*S-1-5-18:(OI)(CI)F"],
        &[&dir_str, "/grant:r", "*S-1-5-32-544:(OI)(CI)F"],
        &[&dir_str, "/grant:r", "*S-1-5-32-545:(OI)(CI)RX"],
    ];
    for args in acl_commands {
        let status = std::process::Command::new("icacls")
            .args(*args)
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .status()
            .map_err(|e| format!("Failed to execute icacls for {:?}: {}", dir, e))?;

        if !status.success() {
            return Err(format!(
                "ACL hardening failed for {:?} with args {:?}: {}",
                dir, args, status
            ));
        }
    }

    Ok(dir.join("search_index.db"))
}

impl IndexDb {
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("SQLite open error: {}", e))?;

        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=10000;")
            .map_err(|e| format!("PRAGMA error: {}", e))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS volume_state (
                drive_letter TEXT PRIMARY KEY,
                journal_id INTEGER NOT NULL,
                last_usn INTEGER NOT NULL,
                files_indexed INTEGER NOT NULL,
                last_full_scan_epoch INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS file_records (
                frn INTEGER NOT NULL,
                drive_letter TEXT NOT NULL,
                name TEXT NOT NULL,
                parent_frn INTEGER NOT NULL,
                is_dir INTEGER NOT NULL,
                PRIMARY KEY (drive_letter, frn)
            );",
        )
        .map_err(|e| format!("Table creation error: {}", e))?;

        Self::migrate_schema(&conn)?;

        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS search_fts USING fts5(
                name,
                content='file_records',
                content_rowid='rowid',
                tokenize='trigram'
            );",
        )
        .map_err(|e| format!("FTS5 table creation error: {}", e))?;

        let has_records: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM file_records LIMIT 1)",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if has_records {
            let start = std::time::Instant::now();
            conn.execute(
                "INSERT INTO search_fts(search_fts) VALUES('rebuild')",
                [],
            )
            .map_err(|e| format!("FTS5 initial rebuild error: {}", e))?;
            eprintln!(
                "[DB] FTS5 index rebuilt at startup in {:.2}s",
                start.elapsed().as_secs_f64()
            );
        }

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn migrate_schema(conn: &Connection) -> Result<(), String> {
        let col_count: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('file_records')",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if col_count == 7 {
            eprintln!("[DB] Migrating file_records from old 7-column schema to compact 5-column schema...");
            conn.execute_batch(
                "DROP TABLE IF EXISTS file_records;
                 CREATE TABLE file_records (
                     frn INTEGER NOT NULL,
                     drive_letter TEXT NOT NULL,
                     name TEXT NOT NULL,
                     parent_frn INTEGER NOT NULL,
                     is_dir INTEGER NOT NULL,
                     PRIMARY KEY (drive_letter, frn)
                 );",
            )
            .map_err(|e| format!("Schema migration error: {}", e))?;
            eprintln!("[DB] Migration complete. Index will be rebuilt on next scan.");
        }

        Ok(())
    }

    /// Load persisted volume state.
    pub fn load_volume_state(&self, drive_letter: char) -> Option<PersistedVolumeState> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT journal_id, last_usn, files_indexed FROM volume_state WHERE drive_letter = ?1",
            )
            .ok()?;

        stmt.query_row(params![drive_letter.to_string()], |row| {
            Ok(PersistedVolumeState {
                drive_letter,
                journal_id: row.get::<_, i64>(0)? as u64,
                last_usn: row.get(1)?,
                files_indexed: row.get::<_, i64>(2)? as u64,
            })
        })
        .ok()
    }

    /// Stream file records from DB directly into the VolumeIndex's arena.
    /// Returns the number of records loaded, or None if no records found.
    pub fn load_into_index(
        &self,
        index: &mut crate::file_index::VolumeIndex,
    ) -> Option<usize> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT frn, name, parent_frn, is_dir
                 FROM file_records WHERE drive_letter = ?1",
            )
            .ok()?;

        let mut count = 0usize;
        let rows = stmt
            .query_map(params![index.drive_letter.to_string()], |row| {
                let frn: i64 = row.get(0)?;
                let name: String = row.get(1)?;
                let parent_frn: i64 = row.get(2)?;
                let is_dir: bool = row.get(3)?;
                Ok((frn as u64, name, parent_frn as u64, is_dir))
            })
            .ok()?;

        for (frn, name, parent_ref, is_dir) in rows.flatten() {
            if !index.insert_record(frn, &name, parent_ref, is_dir) {
                eprintln!("[INDEX-DB] Name arena full — stopping load for volume");
                break;
            }
            count += 1;
        }

        if count == 0 { None } else { Some(count) }
    }
}
