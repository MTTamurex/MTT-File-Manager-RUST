//! Persistent SQLite store for user settings and app state.
//!
//! Manages: user_preferences, folder_locks, pinned_folders, folder_covers.
//! Connection management, ACL hardening, and PRAGMA setup are delegated to
//! `crate::infrastructure::db_utils`.

use rusqlite::Connection;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::db_utils;

mod cleanup;
mod folder_covers;
mod folder_locks;
pub(crate) mod gc;
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
    /// Creates a new app state database at the specified directory.
    pub fn new(state_dir: PathBuf) -> rusqlite::Result<Self> {
        if let Err(e) = fs::create_dir_all(&state_dir) {
            log::warn!(
                "[APP-STATE] Failed to ensure state directory {:?}: {}",
                state_dir,
                e
            );
        }

        let primary_hardened = db_utils::harden_directory_permissions(&state_dir);
        if !primary_hardened {
            log::warn!(
                "[APP-STATE] State directory ACL hardening failed for {:?}",
                state_dir
            );
        }

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
    }
}
