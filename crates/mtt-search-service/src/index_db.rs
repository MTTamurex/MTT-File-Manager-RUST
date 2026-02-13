use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection};

use crate::file_index::VolumeIndex;

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
    let created = std::fs::create_dir_all(&dir);

    // Harden directory permissions: remove inherited ACLs, grant SYSTEM and
    // Administrators full control, grant Users read-only.
    // This prevents non-admin malware from replacing the DB (cache poisoning).
    // icacls is called directly (not via cmd /C) to prevent shell metacharacter injection.
    if created.is_ok() {
        let dir_str = dir.to_string_lossy().to_string();
        let acl_commands: &[&[&str]] = &[
            &[&dir_str, "/inheritance:r"],
            &[&dir_str, "/grant:r", "SYSTEM:(OI)(CI)F"],
            &[&dir_str, "/grant:r", "Administrators:(OI)(CI)F"],
            &[&dir_str, "/grant:r", "Users:(OI)(CI)RX"],
        ];
        for args in acl_commands {
            let _ = std::process::Command::new("icacls")
                .args(*args)
                .creation_flags(0x08000000) // CREATE_NO_WINDOW
                .status();
        }
    }

    dir.join("search_index.db")
}

impl IndexDb {
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("SQLite open error: {}", e))?;

        // Enable WAL mode for better concurrency
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .map_err(|e| format!("PRAGMA error: {}", e))?;

        // Create tables (compact schema — no name_lower, no size)
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

        // Migrate from old schema: if legacy columns exist, drop and recreate
        Self::migrate_schema(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Migrate from old 7-column schema (with name_lower, size) to compact 5-column schema.
    /// Detects the old layout by checking column count via PRAGMA, then recreates the table.
    fn migrate_schema(conn: &Connection) -> Result<(), String> {
        let col_count: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('file_records')",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Old schema had 7 columns (frn, drive_letter, name, name_lower, parent_frn, is_dir, size).
        // New schema has 5 columns.  If we see 7, migrate.
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

    /// Stream file records from DB directly into the VolumeIndex's arena.
    /// Returns the number of records loaded, or None if no records found.
    ///
    /// This avoids creating a temporary `Vec<String>` (~110 MB for 1.5M files)
    /// by inserting each record into the arena as it's read from SQLite.
    pub fn load_into_index(
        &self,
        index: &mut crate::file_index::VolumeIndex,
    ) -> Option<usize> {
        let conn = self.conn.lock().ok()?;
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

        for row in rows {
            if let Ok((frn, name, parent_ref, is_dir)) = row {
                index.insert_record(frn, &name, parent_ref, is_dir);
                count += 1;
                // `name` (String) is dropped here — no memory buildup
            }
        }

        if count == 0 { None } else { Some(count) }
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
                    "INSERT INTO file_records (frn, drive_letter, name, parent_frn, is_dir)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )
                .map_err(|e| format!("Prepare insert error: {}", e))?;

            for (&frn, record) in &index.records {
                let name = index.names.get(record.name_ref());
                insert_stmt
                    .execute(params![
                        frn as i64,
                        drive,
                        name,
                        record.parent_ref as i64,
                        record.is_dir
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
