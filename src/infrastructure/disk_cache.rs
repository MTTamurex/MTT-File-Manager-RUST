//! Persistent SQLite cache for thumbnails
//! Follows .cursorrules: I/O in worker threads, RAII for resources

use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

mod cleanup;
mod folder_covers;
mod folder_locks;
mod folder_previews;
mod gc;
mod pinned_folders;
mod preferences;
mod thumbnails_repo;

/// Allowed table targets for batch-delete operations.
/// Using an enum instead of raw &str prevents SQL injection through
/// table or column names.
#[derive(Clone, Copy)]
enum CacheTable {
    Thumbnails,
    FolderCovers,
    FolderPreviews,
}

impl CacheTable {
    fn table_name(self) -> &'static str {
        match self {
            Self::Thumbnails => "thumbnails",
            Self::FolderCovers => "folder_covers",
            Self::FolderPreviews => "folder_previews",
        }
    }
    fn key_col(self) -> &'static str {
        match self {
            Self::Thumbnails => "id",
            Self::FolderCovers | Self::FolderPreviews => "folder_path",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ThumbnailCacheEntry {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub requested_size: u32,
}

/// Manages persistent thumbnail storage in SQLite
pub struct ThumbnailDiskCache {
    writer: Arc<Mutex<Connection>>, // For put, set_*, garbage_collect (DELETE)
    reader: Arc<Mutex<Connection>>, // For get, get_*, check existence
    #[allow(dead_code)]
    cache_dir: PathBuf,
}

impl ThumbnailDiskCache {
    fn harden_directory_permissions(cache_dir: &Path) -> bool {
        let Ok(username) = std::env::var("USERNAME") else {
            log::warn!(
                "[DISK-CACHE] USERNAME env not available; skipping ACL hardening for {:?}",
                cache_dir
            );
            return false;
        };

        use std::os::windows::process::CommandExt;
        let dir_str = cache_dir.to_string_lossy().to_string();
        let grant_arg = format!("{}:(OI)(CI)F", username);

        for args in [
            vec![dir_str.as_str(), "/inheritance:r"],
            vec![dir_str.as_str(), "/grant:r", grant_arg.as_str()],
        ] {
            match std::process::Command::new("icacls")
                .args(&args)
                .creation_flags(0x08000000) // CREATE_NO_WINDOW
                .status()
            {
                Err(e) => {
                    log::warn!("[DISK-CACHE] icacls failed for {:?}: {}", cache_dir, e);
                    return false;
                }
                Ok(status) if !status.success() => {
                    log::warn!("[DISK-CACHE] icacls exited with {} for {:?}", status, cache_dir);
                    return false;
                }
                Ok(_) => {}
            }
        }

        true
    }

    fn open_temp_fallback_connection(
        temp_fallback_path: &Path,
    ) -> rusqlite::Result<(Connection, Option<PathBuf>)> {
        if let Some(parent) = temp_fallback_path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                log::warn!(
                    "[Cache] Failed to ensure temporary fallback directory {:?}: {}",
                    parent,
                    e
                );
            }
        }

        let temp_parent_hardened = temp_fallback_path
            .parent()
            .map(Self::harden_directory_permissions)
            .unwrap_or(false);

        if !temp_parent_hardened {
            log::warn!(
                "[Cache] Temporary fallback directory ACL hardening failed. Using in-memory cache instead."
            );
            return Ok((Connection::open_in_memory()?, None));
        }

        match Connection::open(temp_fallback_path) {
            Ok(c) => {
                log::warn!(
                    "[Cache] Using temporary fallback database at {:?}",
                    temp_fallback_path
                );
                Ok((c, Some(temp_fallback_path.to_path_buf())))
            }
            Err(temp_err) => {
                log::warn!(
                    "[Cache] Failed to open temporary fallback database: {:?}. Using in-memory cache.",
                    temp_err
                );
                Ok((Connection::open_in_memory()?, None))
            }
        }
    }

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
        let primary_hardened = Self::harden_directory_permissions(&cache_dir);
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
                        Self::open_temp_fallback_connection(&temp_fallback_path)?;
                    (conn, fallback_path)
                }
            }
        } else {
            log::warn!(
                "[Cache] Skipping primary database path due to ACL hardening failure at {:?}",
                cache_dir
            );
            let (conn, fallback_path) = Self::open_temp_fallback_connection(&temp_fallback_path)?;
            (conn, fallback_path)
        };

        // Performance Tuning: Use WAL mode for better concurrency (readers don't block writers)
        // and NORMAL synchronous for faster writes (safe in WAL mode).
        let _ = writer_conn.execute("PRAGMA journal_mode = WAL", []).ok();
        let _ = writer_conn.execute("PRAGMA synchronous = NORMAL", []).ok();

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
            let _ = reader_conn.execute("PRAGMA synchronous = NORMAL", []).ok();
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

        // Create preferences table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS user_preferences (
                key TEXT PRIMARY KEY,
                value TEXT
            )",
            [],
        )
        .unwrap_or(0);

        // Create folder covers table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS folder_covers (
                folder_path TEXT PRIMARY KEY,
                cover_path TEXT
            )",
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

        // Directory index tables
        conn.execute(
            "CREATE TABLE IF NOT EXISTS directory_index (
                dir_path TEXT PRIMARY KEY,
                file_count INTEGER NOT NULL,
                total_size INTEGER NOT NULL,
                last_scan_time INTEGER NOT NULL,
                scan_duration_ms INTEGER NOT NULL
            )",
            [],
        )
        .unwrap_or(0);

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
        )
        .unwrap_or(0);

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_file_index_dir ON file_index(dir_path)",
            [],
        )
        .unwrap_or(0);

        // Folder locks table (per-folder view preferences)
        // Migration: drop legacy table that had a search_query NOT NULL column,
        // which caused INSERT failures (constraint violation error 1299).
        // Since the old INSERT always failed, the table is guaranteed to be empty.
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
