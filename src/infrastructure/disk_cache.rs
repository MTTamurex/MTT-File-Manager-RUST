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
mod shell_icons;
mod thumbnails_repo;

/// SEC: Get the raw SID bytes for the current process user from the process token.
/// Returns a buffer whose prefix is a valid SID structure.
fn get_current_user_sid_bytes() -> Option<(Vec<u8>, u32)> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Security::{
        GetLengthSid, GetTokenInformation, IsValidSid, TokenUser, TOKEN_QUERY, TOKEN_USER,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = windows::Win32::Foundation::HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;

        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        if needed == 0 {
            let _ = CloseHandle(token);
            return None;
        }

        let mut buf = vec![0u8; needed as usize];
        let ok = GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr() as *mut _),
            needed,
            &mut needed,
        );
        let _ = CloseHandle(token);
        ok.ok()?;

        let user_info = &*(buf.as_ptr() as *const TOKEN_USER);
        let sid = user_info.User.Sid;
        if !IsValidSid(sid).as_bool() {
            return None;
        }
        let sid_len = GetLengthSid(sid);
        let sid_ptr = sid.0 as *const u8;
        let sid_bytes = std::slice::from_raw_parts(sid_ptr, sid_len as usize).to_vec();
        Some((sid_bytes, sid_len))
    }
}

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
    /// SEC: Apply an explicit DACL to the cache directory using Win32 API directly.
    /// Grants the current user Full Control with inheritance, and removes inherited
    /// permissions. This replaces the previous icacls subprocess approach, eliminating
    /// the TOCTOU window between directory creation and ACL application.
    fn harden_directory_permissions(cache_dir: &Path) -> bool {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Foundation::LocalFree;
        use windows::Win32::Security::Authorization::{
            SetNamedSecurityInfoW, SE_FILE_OBJECT, SET_ACCESS,
            SetEntriesInAclW, EXPLICIT_ACCESS_W, TRUSTEE_W,
            TRUSTEE_IS_SID, TRUSTEE_IS_USER,
        };
        use windows::Win32::Security::{
            ACL as WIN_ACL, ACE_FLAGS,
            DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        };

        // Get the current user's raw SID from the process token.
        let Some((mut user_sid_bytes, _sid_len)) = get_current_user_sid_bytes() else {
            log::warn!(
                "[DISK-CACHE] Failed to get current user SID; skipping ACL hardening for {:?}",
                cache_dir
            );
            return false;
        };

        // FILE_ALL_ACCESS = Full Control for the owner.
        const FILE_ALL_ACCESS: u32 = 0x001F01FF;

        // CONTAINER_INHERIT_ACE | OBJECT_INHERIT_ACE = sub-containers and objects inherit.
        let inheritance = ACE_FLAGS(3u32);

        let entries = [
            EXPLICIT_ACCESS_W {
                grfAccessPermissions: FILE_ALL_ACCESS,
                grfAccessMode: SET_ACCESS,
                grfInheritance: inheritance,
                Trustee: TRUSTEE_W {
                    TrusteeForm: TRUSTEE_IS_SID,
                    TrusteeType: TRUSTEE_IS_USER,
                    ptstrName: windows::core::PWSTR(user_sid_bytes.as_mut_ptr() as *mut u16),
                    ..Default::default()
                },
            },
        ];

        // Build the new ACL from the explicit entry.
        let mut new_acl = std::ptr::null_mut::<WIN_ACL>();
        let result = unsafe { SetEntriesInAclW(Some(&entries), None, &mut new_acl) };
        if result.0 != 0 {
            log::warn!(
                "[DISK-CACHE] SetEntriesInAclW failed with code {} for {:?}",
                result.0,
                cache_dir
            );
            return false;
        }

        // Apply the ACL to the directory. PROTECTED_DACL_SECURITY_INFORMATION removes
        // inherited ACEs (equivalent to `icacls /inheritance:r`).
        let dir_wide: Vec<u16> = OsStr::new(cache_dir.as_os_str())
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let set_result = unsafe {
            SetNamedSecurityInfoW(
                windows::core::PCWSTR(dir_wide.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(new_acl as *const _),
                None,
            )
        };

        // Free the ACL allocated by SetEntriesInAclW.
        if !new_acl.is_null() {
            unsafe {
                LocalFree(Some(
                    windows::Win32::Foundation::HLOCAL(new_acl as *mut _),
                ));
            }
        }

        if set_result.0 != 0 {
            log::warn!(
                "[DISK-CACHE] SetNamedSecurityInfoW failed with code {} for {:?}",
                set_result.0,
                cache_dir
            );
            return false;
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
