//! Persistent SQLite store for user settings and app state.
//!
//! Manages: user_preferences, folder_locks, pinned_folders, folder_covers.
//! Connection management, ACL hardening, and PRAGMA setup are delegated to
//! `crate::infrastructure::db_utils`.

use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::db_utils;

mod cleanup;
mod file_entry_cache;
mod file_tags;
mod folder_covers;
mod folder_locks;
pub(crate) mod gc;
mod organizer_rules;
mod pinned_folders;
mod preferences;

/// Persistent store for user settings and metadata.
///
/// Tables: user_preferences, folder_locks, pinned_folders, folder_covers.
/// Uses the same dual writer/reader + WAL pattern as `ThumbnailDiskCache`.
pub struct AppStateDb {
    writer: Arc<Mutex<Connection>>,
    reader: Arc<Mutex<Connection>>,
    #[allow(dead_code)]
    state_dir: PathBuf,
}

impl AppStateDb {
    const ACL_HARDENED_MARKER: &'static str = ".acl_hardened";

    fn acl_marker_path(state_dir: &Path) -> PathBuf {
        state_dir.join(Self::ACL_HARDENED_MARKER)
    }

    /// Creates a new app state database at the specified directory.
    pub fn new(state_dir: PathBuf) -> rusqlite::Result<Self> {
        if let Err(e) = fs::create_dir_all(&state_dir) {
            log::warn!(
                "[APP-STATE] Failed to ensure state directory {:?}: {}",
                state_dir,
                e
            );
        }

        // PERF: Skip ACL hardening if marker file exists (same pattern as ThumbnailDiskCache).
        // SetNamedSecurityInfoW is an LSASS round-trip that costs ~30-80ms on cold start.
        let primary_hardened = if Self::acl_marker_path(&state_dir).exists() {
            true
        } else {
            let hardened = db_utils::harden_directory_permissions(&state_dir);
            if hardened {
                let _ = fs::write(Self::acl_marker_path(&state_dir), b"1");
            }
            if !hardened {
                log::warn!(
                    "[APP-STATE] State directory ACL hardening failed for {:?}",
                    state_dir
                );
            }
            hardened
        };

        let db_path = state_dir.join("app_state.db");
        let temp_fallback_path = std::env::temp_dir()
            .join("MTT-File-Manager")
            .join("app_state_fallback.db");

        // 1. Open WRITER connection (Primary -> Temp fallback -> Memory fallback)
        let (writer_conn, active_db_path) = if primary_hardened {
            match Connection::open(&db_path) {
                Ok(c) => (c, Some(db_path.clone())),
                Err(primary_err) => {
                    log::warn!(
                        "[APP-STATE] Failed to open database at {:?}: {:?}",
                        db_path,
                        primary_err
                    );
                    let (conn, fallback_path) =
                        db_utils::open_temp_fallback_connection(&temp_fallback_path)?;
                    (conn, fallback_path)
                }
            }
        } else {
            log::warn!(
                "[APP-STATE] Skipping primary path due to ACL hardening failure at {:?}",
                state_dir
            );
            let (conn, fallback_path) =
                db_utils::open_temp_fallback_connection(&temp_fallback_path)?;
            (conn, fallback_path)
        };

        db_utils::apply_default_pragmas(&writer_conn);

        // 2. Open READER connection
        let reader_conn = if let Some(path) = active_db_path.as_ref() {
            match Connection::open(path) {
                Ok(c) => Some(c),
                Err(e) => {
                    log::warn!(
                        "[APP-STATE] Failed to open reader connection at {:?}: {:?}. Sharing writer.",
                        path, e
                    );
                    None
                }
            }
        } else {
            None
        };

        // 3. Schema Migrations
        Self::run_migrations(&writer_conn);

        let writer = Arc::new(Mutex::new(writer_conn));
        let reader = if let Some(reader_conn) = reader_conn {
            db_utils::apply_default_pragmas(&reader_conn);
            Arc::new(Mutex::new(reader_conn))
        } else {
            writer.clone()
        };

        Ok(Self {
            writer,
            reader,
            state_dir,
        })
    }

    fn run_migrations(conn: &Connection) {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS user_preferences (
                key TEXT PRIMARY KEY,
                value TEXT
            )",
            [],
        )
        .unwrap_or(0);

        conn.execute(
            "CREATE TABLE IF NOT EXISTS folder_covers (
                folder_path TEXT PRIMARY KEY,
                cover_path TEXT
            )",
            [],
        )
        .unwrap_or(0);

        // Folder locks table (per-folder view preferences)
        // Migration: drop legacy table that had a search_query NOT NULL column,
        // which caused INSERT failures (constraint violation error 1299).
        let has_search_query_col = conn
            .prepare("SELECT search_query FROM folder_locks LIMIT 0")
            .is_ok();
        if has_search_query_col {
            conn.execute("DROP TABLE folder_locks", []).unwrap_or(0);
        }
        conn.execute(
            "CREATE TABLE IF NOT EXISTS folder_locks (
                path TEXT PRIMARY KEY,
                view_mode TEXT NOT NULL,
                sort_mode TEXT NOT NULL,
                sort_descending TEXT NOT NULL,
                folders_position TEXT NOT NULL
            )",
            [],
        )
        .unwrap_or(0);

        // Quick Access pinned folders table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS pinned_folders (
                path TEXT PRIMARY KEY,
                display_name TEXT NOT NULL,
                position INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )
        .unwrap_or(0);

        conn.execute(
            "CREATE TABLE IF NOT EXISTS file_tags (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL COLLATE NOCASE UNIQUE,
                color TEXT NOT NULL,
                position INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )
        .unwrap_or(0);

        conn.execute(
            "CREATE TABLE IF NOT EXISTS file_tag_assignments (
                file_path TEXT NOT NULL COLLATE NOCASE,
                tag_id INTEGER NOT NULL,
                PRIMARY KEY (file_path, tag_id),
                FOREIGN KEY (tag_id) REFERENCES file_tags(id) ON DELETE CASCADE
            )",
            [],
        )
        .unwrap_or(0);

        Self::migrate_file_tag_assignments_to_nocase(conn);

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_file_tag_assignments_tag
             ON file_tag_assignments(tag_id)",
            [],
        )
        .unwrap_or(0);

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_file_tag_assignments_tag_path
             ON file_tag_assignments(tag_id, file_path)",
            [],
        )
        .unwrap_or(0);

        // File entry cache (persistent metadata for tag views).
        // Used to skip GetFileAttributesExW syscalls on tag selection,
        // especially on a cold NTFS cache after restart or long idle.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS file_entry_cache (
                file_path TEXT PRIMARY KEY COLLATE NOCASE,
                is_dir INTEGER NOT NULL,
                size INTEGER NOT NULL,
                modified INTEGER NOT NULL,
                created INTEGER,
                is_hidden INTEGER NOT NULL,
                sync_status INTEGER NOT NULL,
                cached_at INTEGER NOT NULL
            )",
            [],
        )
        .unwrap_or(0);

        conn.execute(
            "CREATE TABLE IF NOT EXISTS organizer_rules (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_folder TEXT NOT NULL COLLATE NOCASE,
                destination_folder TEXT NOT NULL COLLATE NOCASE,
                extensions TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1
            )",
            [],
        )
        .unwrap_or(0);

        file_tags::seed_default_file_tags(conn);
    }

    fn migrate_file_tag_assignments_to_nocase(conn: &Connection) {
        let create_sql = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'file_tag_assignments'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap_or_default();

        if create_sql
            .to_ascii_uppercase()
            .contains("FILE_PATH TEXT NOT NULL COLLATE NOCASE")
        {
            return;
        }

        let migration = (|| -> rusqlite::Result<usize> {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "ALTER TABLE file_tag_assignments RENAME TO file_tag_assignments_old",
                [],
            )?;
            tx.execute(
                "CREATE TABLE file_tag_assignments (
                    file_path TEXT NOT NULL COLLATE NOCASE,
                    tag_id INTEGER NOT NULL,
                    PRIMARY KEY (file_path, tag_id),
                    FOREIGN KEY (tag_id) REFERENCES file_tags(id) ON DELETE CASCADE
                )",
                [],
            )?;

            let rows_to_insert = {
                let mut stmt = tx.prepare(
                    "SELECT replace(file_path, '/', '\\'), tag_id
                     FROM file_tag_assignments_old
                     ORDER BY rowid ASC",
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })?;

                let mut canonical_by_key = std::collections::HashMap::<String, String>::new();
                let mut rows_to_insert = Vec::new();
                for row in rows {
                    let (path, tag_id) = row?;
                    let key = path.to_lowercase();
                    let canonical_path = canonical_by_key
                        .entry(key)
                        .or_insert_with(|| path.clone())
                        .clone();
                    rows_to_insert.push((canonical_path, tag_id));
                }
                rows_to_insert
            };

            let mut inserted = 0usize;
            {
                let mut insert_stmt = tx.prepare(
                    "INSERT OR IGNORE INTO file_tag_assignments (file_path, tag_id)
                     VALUES (?1, ?2)",
                )?;
                for (path, tag_id) in rows_to_insert {
                    inserted += insert_stmt.execute(params![path, tag_id])?;
                }
            }

            tx.execute("DROP TABLE file_tag_assignments_old", [])?;
            tx.commit()?;
            Ok(inserted)
        })();

        match migration {
            Ok(inserted) => log::info!(
                "[APP-STATE] Migrated file_tag_assignments to NOCASE path keys ({} rows)",
                inserted
            ),
            Err(error) => log::error!(
                "[APP-STATE] Failed to migrate file_tag_assignments to NOCASE path keys: {:?}",
                error
            ),
        }
    }
}
