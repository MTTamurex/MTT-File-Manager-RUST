use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection};

use crate::file_index::{FileRecord, VolumeIndex};

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
pub fn get_db_path() -> PathBuf {
    // Use %PROGRAMDATA%\MTT-File-Manager\search_index.db
    let base = std::env::var("PROGRAMDATA").unwrap_or_else(|_| r"C:\ProgramData".to_string());
    let dir = Path::new(&base).join("MTT-File-Manager");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("search_index.db")
}

impl IndexDb {
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("SQLite open error: {}", e))?;

        // Enable WAL mode for better concurrency
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .map_err(|e| format!("PRAGMA error: {}", e))?;

        // Create tables
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
                name_lower TEXT NOT NULL,
                parent_frn INTEGER NOT NULL,
                is_dir INTEGER NOT NULL,
                size INTEGER NOT NULL,
                PRIMARY KEY (drive_letter, frn)
            );",
        )
        .map_err(|e| format!("Table creation error: {}", e))?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Load persisted volume state.
    pub fn load_volume_state(&self, drive_letter: char) -> Option<PersistedVolumeState> {
        let conn = self.conn.lock().ok()?;
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

    /// Load all file records for a volume.
    pub fn load_file_records(&self, drive_letter: char) -> Option<HashMap<u64, FileRecord>> {
        let conn = self.conn.lock().ok()?;
        let mut stmt = conn
            .prepare(
                "SELECT frn, name, name_lower, parent_frn, is_dir, size
                 FROM file_records WHERE drive_letter = ?1",
            )
            .ok()?;

        let mut records = HashMap::new();
        let rows = stmt
            .query_map(params![drive_letter.to_string()], |row| {
                let frn: i64 = row.get(0)?;
                let name: String = row.get(1)?;
                let name_lower: String = row.get(2)?;
                let parent_frn: i64 = row.get(3)?;
                let is_dir: bool = row.get(4)?;
                let size: i64 = row.get(5)?;
                Ok((
                    frn as u64,
                    FileRecord {
                        name,
                        name_lower,
                        parent_ref: parent_frn as u64,
                        is_dir,
                        size: size as u64,
                    },
                ))
            })
            .ok()?;

        for row in rows {
            if let Ok((frn, record)) = row {
                records.insert(frn, record);
            }
        }

        if records.is_empty() {
            None
        } else {
            Some(records)
        }
    }

    /// Save the complete volume index to the database.
    pub fn save_volume(&self, index: &VolumeIndex) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;

        let drive = index.drive_letter.to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // Update volume state
        conn.execute(
            "INSERT OR REPLACE INTO volume_state
             (drive_letter, journal_id, last_usn, files_indexed, last_full_scan_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                drive,
                index.journal_id as i64,
                index.last_usn,
                index.records.len() as i64,
                now
            ],
        )
        .map_err(|e| format!("Save volume_state error: {}", e))?;

        // Clear old records for this volume and insert new ones in a transaction
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("Transaction begin error: {}", e))?;

        tx.execute(
            "DELETE FROM file_records WHERE drive_letter = ?1",
            params![drive],
        )
        .map_err(|e| format!("Delete old records error: {}", e))?;

        {
            let mut insert_stmt = tx
                .prepare(
                    "INSERT INTO file_records (frn, drive_letter, name, name_lower, parent_frn, is_dir, size)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                )
                .map_err(|e| format!("Prepare insert error: {}", e))?;

            for (&frn, record) in &index.records {
                insert_stmt
                    .execute(params![
                        frn as i64,
                        drive,
                        record.name,
                        record.name_lower,
                        record.parent_ref as i64,
                        record.is_dir,
                        record.size as i64
                    ])
                    .map_err(|e| format!("Insert record error: {}", e))?;
            }
        }

        tx.commit()
            .map_err(|e| format!("Transaction commit error: {}", e))?;

        eprintln!(
            "[DB] Saved {} records for volume {}:\\",
            index.records.len(),
            index.drive_letter
        );
        Ok(())
    }
}
