//! Persistent SQLite cache for thumbnails, folder previews, and shell icons.
//!
//! Connection management, ACL hardening, and PRAGMA setup are delegated to
//! `crate::infrastructure::db_utils`.

use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::db_utils;

mod cleanup;
mod folder_previews;
mod gc;
mod shell_icons;
mod thumbnails_repo;

/// Allowed table targets for batch-delete operations.
/// Using an enum instead of raw &str prevents SQL injection through
/// table or column names.
#[derive(Clone, Copy)]
enum CacheTable {
    Thumbnails,
    FolderPreviews,
}

impl CacheTable {
    fn table_name(self) -> &'static str {
        match self {
            Self::Thumbnails => "thumbnails",
            Self::FolderPreviews => "folder_previews",
        }
    }
    fn key_col(self) -> &'static str {
        match self {
            Self::Thumbnails => "id",
            Self::FolderPreviews => "folder_path",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ThumbnailCacheEntry {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub requested_size: u32,
    /// The `modified_at` epoch stored in the DB row.  Used by callers to
    /// detect stale fallback results from `get_latest`.
    pub modified_at: u64,
}

impl ThumbnailCacheEntry {
    /// Returns true when this cached entry can satisfy a request for `req_size`.
    pub fn satisfies_request(&self, req_size: u32) -> bool {
        let cached_max_dim = self.width.max(self.height);
        if cached_max_dim == 0 {
            return false;
        }

        cached_max_dim >= req_size || self.requested_size >= req_size
    }
}

/// Manages persistent thumbnail storage in SQLite
pub struct ThumbnailDiskCache {
    writer: Arc<Mutex<Connection>>, // For put, set_*, garbage_collect (DELETE)
    reader: Arc<Mutex<Connection>>, // For get, get_*, check existence
    #[allow(dead_code)]
    cache_dir: PathBuf,
}

impl ThumbnailDiskCache {
    /// Creates a new disk cache at the specified directory
    pub fn new(cache_dir: PathBuf) -> rusqlite::Result<Self> {
        // Ensure directory exists
        if let Err(e) = fs::create_dir_all(&cache_dir) {
            log::warn!(
                "[DISK-CACHE] Failed to ensure cache directory {:?}: {}",
                cache_dir,
                e
            );
        }

        // Harden directory permissions on first creation: restrict to owner
        // to prevent cache poisoning by other local users.
        let primary_hardened = db_utils::harden_directory_permissions(&cache_dir);
        if !primary_hardened {
            log::warn!(
                "[DISK-CACHE] Primary cache directory ACL hardening failed for {:?}",
                cache_dir
            );
        }

        // Clean up legacy files if they exist (Migration)
        Self::cleanup_legacy(&cache_dir);

        let db_path = cache_dir.join("thumbnails.db");
        let temp_fallback_path = std::env::temp_dir()
            .join("MTT-File-Manager")
            .join("thumbnails_fallback.db");

        // 1. Open WRITER connection (Primary -> Temp fallback -> Memory fallback)
        let (writer_conn, active_db_path) = if primary_hardened {
            match Connection::open(&db_path) {
                Ok(c) => {
                    (c, Some(db_path.clone()))
                }
                Err(primary_err) => {
                    log::warn!(
                        "[Cache] Failed to open database at {:?}: {:?}",
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
                "[Cache] Skipping primary database path due to ACL hardening failure at {:?}",
                cache_dir
            );
            let (conn, fallback_path) =
                db_utils::open_temp_fallback_connection(&temp_fallback_path)?;
            (conn, fallback_path)
        };

        // Performance Tuning: Use WAL mode for better concurrency (readers don't block writers)
        // and NORMAL synchronous for faster writes (safe in WAL mode).
        db_utils::apply_default_pragmas(&writer_conn);

        // 2. Open READER connection (Secondary)
        // In WAL mode, this can read while writer is busy. If reader cannot be opened,
        // we safely share the writer connection.
        let reader_conn = if let Some(path) = active_db_path.as_ref() {
            match Connection::open(path) {
                Ok(c) => Some(c),
                Err(e) => {
                    log::warn!(
                        "[Cache] Failed to open reader connection at {:?}: {:?}. Falling back to shared writer connection.",
                        path, e
                    );
                    None
                }
            }
        } else {
            // Writer is in-memory fallback: share writer connection to keep consistency.
            None
        };

        // 3. Schema Migrations (Run on Writer)
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
            cache_dir,
        })
    }

    fn run_migrations(conn: &Connection) {
        // Create table (with path for GC)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS thumbnails (
                id TEXT PRIMARY KEY,
                path TEXT,
                data BLOB,
                modified_at INTEGER,
                created_at INTEGER,
                width INTEGER DEFAULT 0,
                height INTEGER DEFAULT 0
            )",
            [],
        )
        .unwrap_or_else(|e| {
            log::warn!("[Cache] Failed to create thumbnails table: {:?}", e);
            0
        });

        // Migration: Add path column if missing
        let _ = conn.execute("ALTER TABLE thumbnails ADD COLUMN path TEXT", []);

        // Migration: Add width and height columns if missing
        let _ = conn.execute(
            "ALTER TABLE thumbnails ADD COLUMN width INTEGER DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE thumbnails ADD COLUMN height INTEGER DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE thumbnails ADD COLUMN requested_size INTEGER DEFAULT 0",
            [],
        );

        // OPTIMIZATION: Index on path to speed up directory clearing
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_thumbnails_path ON thumbnails(path)",
            [],
        )
        .unwrap_or(0);

        // Folder preview cache table (Shell sandwich icons)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS folder_previews (
                folder_path TEXT PRIMARY KEY,
                data BLOB NOT NULL,
                width INTEGER NOT NULL,
                height INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            )",
            [],
        )
        .unwrap_or(0);

        // Shell icon cache (special folders, drives, computer, recycle bin)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS shell_icons (
                key TEXT PRIMARY KEY,
                data BLOB NOT NULL,
                width INTEGER NOT NULL,
                height INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            )",
            [],
        )
        .unwrap_or(0);
    }

    /// Migration utility: removes old folder-based cache
    fn cleanup_legacy(cache_dir: &Path) {
        if let Ok(entries) = fs::read_dir(cache_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    // Our legacy dirs were 2-char hex prefixes (aa, ab, 01...)
                    if name.len() == 2 && name.chars().all(|c| c.is_ascii_hexdigit()) {
                        let _ = fs::remove_dir_all(path);
                    }
                }
            }
        }
    }
}
