use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection};

use crate::file_index::VolumeIndex;

const FTS_READER_BUSY_TIMEOUT_MS: u64 = 2_000;

/// A single FTS5 search match.
pub struct FtsMatch {
    pub frn: u64,
    pub drive_letter: char,
    pub name: String,
    pub is_dir: bool,
}

/// Read-only searcher for FTS5 queries.
///
/// Opens a **separate** SQLite connection in read-only mode so FTS5 queries
/// never contend with the writer (`IndexDb`). WAL mode allows both to operate
/// concurrently.
pub struct FtsSearcher {
    db_path: PathBuf,
}

impl FtsSearcher {
    pub fn open(path: &Path) -> Result<Self, String> {
        Self::open_read_connection(path)?;
        Ok(Self {
            db_path: path.to_path_buf(),
        })
    }

    fn open_read_connection(path: &Path) -> Result<Connection, String> {
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("FTS searcher open error: {}", e))?;
        // WAL is already set by the writer; this is a no-op but explicit.
        // busy_timeout lets the reader retry for up to 10s instead of failing
        // immediately with SQLITE_BUSY when the writer is rebuilding FTS5.
        conn.execute_batch(&format!(
            "PRAGMA journal_mode=WAL; PRAGMA busy_timeout={};",
            FTS_READER_BUSY_TIMEOUT_MS
        ))
            .map_err(|e| format!("FTS searcher PRAGMA error: {}", e))?;
        Ok(conn)
    }

    /// Query FTS5 for file names matching `query` (substring match via trigram tokenizer).
    ///
    /// Multi-word queries require ALL words to appear as substrings (implicit AND).
    /// Returns up to `limit` results starting at `offset`.
    pub fn search(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<FtsMatch>, String> {
        if query.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let fts_query = build_fts5_query(query);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }

        // Open a short-lived read-only connection per request so concurrent
        // FTS queries do not serialize behind a single shared mutex.
        let conn = Self::open_read_connection(&self.db_path)?;

        let mut stmt = conn
            .prepare_cached(
                "SELECT r.frn, r.drive_letter, r.name, r.is_dir
                 FROM search_fts f
                 JOIN file_records r ON r.rowid = f.rowid
                 WHERE search_fts MATCH ?1
                 LIMIT ?2 OFFSET ?3",
            )
            .map_err(|e| format!("FTS search prepare error: {}", e))?;

        let rows = stmt
            .query_map(
                params![fts_query, limit as i64, offset as i64],
                |row| {
                    let frn: i64 = row.get(0)?;
                    let drive_letter: String = row.get(1)?;
                    let name: String = row.get(2)?;
                    let is_dir: bool = row.get(3)?;
                    Ok(FtsMatch {
                        frn: frn as u64,
                        drive_letter: drive_letter.chars().next().unwrap_or('C'),
                        name,
                        is_dir,
                    })
                },
            )
            .map_err(|e| format!("FTS search query error: {}", e))?;

        let mut results = Vec::with_capacity(limit.min(1024));
        for row in rows {
            match row {
                Ok(m) => results.push(m),
                Err(e) => {
                    eprintln!("[FTS] Error reading search result: {}", e);
                }
            }
        }

        Ok(results)
    }
}

/// Build an FTS5 query string for the trigram tokenizer.
///
/// Each whitespace-delimited token is quoted (implicit AND in FTS5).
/// Example: `"report" "xlsx"` matches names containing both substrings.
fn build_fts5_query(query: &str) -> String {
    let tokens: Vec<&str> = query.split_whitespace().collect();
    if tokens.is_empty() {
        return String::new();
    }
    tokens
        .iter()
        .map(|t| {
            // Escape double quotes inside the token for FTS5 syntax.
            let escaped = t.replace('"', "\"\"");
            format!("\"{}\"", escaped)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

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
    // Use %PROGRAMDATA%\MTT-File-Manager\search_index.db
    let base = std::env::var("PROGRAMDATA").unwrap_or_else(|_| r"C:\ProgramData".to_string());
    let dir = Path::new(&base).join("MTT-File-Manager");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create ProgramData directory {:?}: {}", dir, e))?;

    // Harden directory permissions: remove inherited ACLs, grant SYSTEM and
    // Administrators full control, grant Users read-only.
    // This prevents non-admin malware from replacing the DB (cache poisoning).
    // icacls is called directly (not via cmd /C) to prevent shell metacharacter injection.
    let dir_str = dir.to_string_lossy().to_string();
    let acl_commands: &[&[&str]] = &[
        &[&dir_str, "/inheritance:r"],
        // Use SID-based grants to avoid localization failures (e.g. non-English
        // Windows where group names like "Administrators" are not resolvable).
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

        // Enable WAL mode for better concurrency.
        // busy_timeout lets concurrent operations retry for up to 10s instead of
        // failing immediately with SQLITE_BUSY.
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=10000;")
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

        // FTS5 virtual table for fast substring search using trigram tokenizer.
        // content='file_records' means FTS5 reads text from the existing table
        // (no data duplication). Requires explicit `rebuild` after bulk changes.
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS search_fts USING fts5(
                name,
                content='file_records',
                content_rowid='rowid',
                tokenize='trigram'
            );",
        )
        .map_err(|e| format!("FTS5 table creation error: {}", e))?;

        // If file_records already has data (cached from previous service run),
        // rebuild FTS5 now so searches work immediately after restart.
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

        for (frn, name, parent_ref, is_dir) in rows.flatten() {
            if !index.insert_record(frn, &name, parent_ref, is_dir) {
                eprintln!("[INDEX-DB] Name arena full — stopping load for volume");
                break;
            }
            count += 1;
            // `name` (String) is dropped here — no memory buildup
        }

        if count == 0 { None } else { Some(count) }
    }

    /// Save the complete volume index to the database.
    ///
    /// This is expensive for large volumes (DELETE ALL + INSERT ALL + FTS5 rebuild).
    /// Use only for initial scan or service shutdown.  For periodic persist, prefer
    /// `save_volume_state` + `sync_fts_incremental`.
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

        // Rebuild FTS5 index from the updated file_records.
        // This runs outside the transaction so readers see the new records.
        let fts_start = std::time::Instant::now();
        conn.execute(
            "INSERT INTO search_fts(search_fts) VALUES('rebuild')",
            [],
        )
        .map_err(|e| format!("FTS5 rebuild error after save: {}", e))?;

        eprintln!(
            "[DB] Saved {} records for volume {}:\\ (FTS5 rebuilt in {:.2}s)",
            index.records.len(),
            index.drive_letter,
            fts_start.elapsed().as_secs_f64()
        );
        Ok(())
    }

    pub fn save_volume_state_snapshot(
        &self,
        drive_letter: char,
        journal_id: u64,
        last_usn: i64,
        files_indexed: usize,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;

        let drive = drive_letter.to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        conn.execute(
            "INSERT OR REPLACE INTO volume_state
             (drive_letter, journal_id, last_usn, files_indexed, last_full_scan_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![drive, journal_id as i64, last_usn, files_indexed as i64, now],
        )
        .map_err(|e| format!("Save volume_state error: {}", e))?;

        Ok(())
    }

    pub fn sync_fts_incremental_snapshot(
        &self,
        drive_letter: char,
        additions: &[(u64, String, u64, bool)],
        removals: &std::collections::HashSet<u64>,
    ) -> Result<(), String> {
        if additions.is_empty() && removals.is_empty() {
            return Ok(());
        }

        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let drive = drive_letter.to_string();

        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("Transaction begin error: {}", e))?;

        let mut removed_count = 0usize;
        let mut added_count = 0usize;
        let mut updated_count = 0usize;

        // --- Process removals ---
        // For content-sync FTS5, we must tell FTS5 about the deletion BEFORE
        // we delete the row from file_records.
        for &frn in removals {
            // Look up the existing rowid + name so we can remove the FTS5 entry.
            let existing: Option<(i64, String)> = tx
                .query_row(
                    "SELECT rowid, name FROM file_records WHERE drive_letter = ?1 AND frn = ?2",
                    params![drive, frn as i64],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok();

            if let Some((rowid, old_name)) = existing {
                // Remove from FTS5 first.
                let _ = tx.execute(
                    "INSERT INTO search_fts(search_fts, rowid, name) VALUES('delete', ?1, ?2)",
                    params![rowid, old_name],
                );
                // Remove from file_records.
                tx.execute(
                    "DELETE FROM file_records WHERE drive_letter = ?1 AND frn = ?2",
                    params![drive, frn as i64],
                )
                .map_err(|e| format!("Delete record error: {}", e))?;
                removed_count += 1;
            }
        }

        // --- Process additions (new + updated records) ---
        for (frn, name, parent_ref, is_dir) in additions {
            let frn = *frn;
            let parent_ref = *parent_ref;
            let is_dir = *is_dir;

            // Check if row already exists (update/rename case).
            let existing: Option<(i64, String)> = tx
                .query_row(
                    "SELECT rowid, name FROM file_records WHERE drive_letter = ?1 AND frn = ?2",
                    params![drive, frn as i64],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok();

            if let Some((rowid, old_name)) = existing {
                // Update existing row (preserves rowid).
                tx.execute(
                    "UPDATE file_records SET name = ?1, parent_frn = ?2, is_dir = ?3
                     WHERE drive_letter = ?4 AND frn = ?5",
                    params![name, parent_ref as i64, is_dir, drive, frn as i64],
                )
                .map_err(|e| format!("Update record error: {}", e))?;

                // Re-sync FTS5: remove old entry, add updated entry (same rowid).
                let _ = tx.execute(
                    "INSERT INTO search_fts(search_fts, rowid, name) VALUES('delete', ?1, ?2)",
                    params![rowid, old_name],
                );
                let _ = tx.execute(
                    "INSERT INTO search_fts(rowid, name) VALUES(?1, ?2)",
                    params![rowid, name],
                );
                updated_count += 1;
            } else {
                // New record — insert into file_records, then FTS5.
                tx.execute(
                    "INSERT INTO file_records (frn, drive_letter, name, parent_frn, is_dir)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![frn as i64, drive, name, parent_ref as i64, is_dir],
                )
                .map_err(|e| format!("Insert record error: {}", e))?;

                let new_rowid = tx.last_insert_rowid();
                let _ = tx.execute(
                    "INSERT INTO search_fts(rowid, name) VALUES(?1, ?2)",
                    params![new_rowid, name],
                );
                added_count += 1;
            }
        }

        tx.commit()
            .map_err(|e| format!("Transaction commit error: {}", e))?;

        if removed_count > 0 || added_count > 0 || updated_count > 0 {
            eprintln!(
                "[DB] {}:\\ Incremental sync: +{} ~{} -{} records",
                drive_letter, added_count, updated_count, removed_count
            );
        }

        Ok(())
    }
}
