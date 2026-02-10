//! Persistent SQLite cache for thumbnails
//! Follows .cursorrules: I/O in worker threads, RAII for resources

use image::{DynamicImage, ImageBuffer, Rgba};
use rusqlite::{params, Connection};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

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
    /// Creates a new disk cache at the specified directory
    pub fn new(cache_dir: PathBuf) -> rusqlite::Result<Self> {
        // Ensure directory exists
        if !cache_dir.exists() {
            let _ = fs::create_dir_all(&cache_dir);
        }

        // Clean up legacy files if they exist (Migration)
        Self::cleanup_legacy(&cache_dir);

        let db_path = cache_dir.join("thumbnails.db");
        let temp_fallback_path = std::env::temp_dir()
            .join("MTT-File-Manager")
            .join("thumbnails_fallback.db");

        let mut active_db_path: Option<PathBuf> = None;

        // 1. Open WRITER connection (Primary -> Temp fallback -> Memory fallback)
        let writer_conn = match Connection::open(&db_path) {
            Ok(c) => {
                active_db_path = Some(db_path.clone());
                c
            }
            Err(primary_err) => {
                eprintln!(
                    "[Cache] Failed to open database at {:?}: {:?}",
                    db_path, primary_err
                );

                if let Some(parent) = temp_fallback_path.parent() {
                    let _ = fs::create_dir_all(parent);
                }

                match Connection::open(&temp_fallback_path) {
                    Ok(c) => {
                        eprintln!(
                            "[Cache] Using temporary fallback database at {:?}",
                            temp_fallback_path
                        );
                        active_db_path = Some(temp_fallback_path.clone());
                        c
                    }
                    Err(temp_err) => {
                        eprintln!(
                            "[Cache] Failed to open temporary fallback database: {:?}. Using in-memory cache.",
                            temp_err
                        );
                        Connection::open_in_memory()?
                    }
                }
            }
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
                    eprintln!(
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
            eprintln!(
                "[Cache] Warning: Failed to create thumbnails table: {:?}",
                e
            );
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

    /// Generates a unique hash for a file path
    fn hash_path(path: &Path) -> String {
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    /// Tries to retrieve a thumbnail from SQLite with dimensions and request metadata.
    /// [READER] concurrency friendly
    pub fn get(&self, path: &Path, modified: SystemTime) -> Option<ThumbnailCacheEntry> {
        let id = Self::hash_path(path);
        let mod_time = modified
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let db = self.reader.lock().ok()?;
        let mut stmt = db
            .prepare_cached(
                "SELECT data, width, height, requested_size
                 FROM thumbnails
                 WHERE id = ? AND modified_at = ?",
            )
            .ok()?;

        stmt.query_row(params![id, mod_time], |row| {
            Ok(ThumbnailCacheEntry {
                data: row.get(0)?,
                width: row.get::<_, i64>(1)? as u32,
                height: row.get::<_, i64>(2)? as u32,
                requested_size: row.get::<_, i64>(3)? as u32,
            })
        })
        .ok()
    }

    /// Retrieves the latest thumbnail entry for a path, ignoring modified time.
    /// Useful for virtual filesystems where reported mtime can be unstable.
    /// [READER] concurrency friendly
    pub fn get_latest(&self, path: &Path) -> Option<ThumbnailCacheEntry> {
        let id = Self::hash_path(path);
        let db = self.reader.lock().ok()?;

        // DEBUG: Check total row count for this id
        let count: i64 = db
            .prepare_cached("SELECT COUNT(*) FROM thumbnails WHERE id = ?")
            .ok()
            .and_then(|mut s| s.query_row(params![id], |r| r.get(0)).ok())
            .unwrap_or(-1);
        if count == 0 {
            eprintln!(
                "[DB-MISS] get_latest: id={} path={:?} → 0 rows in DB",
                &id[..8],
                path.file_name()
            );
        }

        let mut stmt = db
            .prepare_cached(
                "SELECT data, width, height, requested_size
                 FROM thumbnails
                 WHERE id = ?",
            )
            .ok()?;

        stmt.query_row(params![id], |row| {
            Ok(ThumbnailCacheEntry {
                data: row.get(0)?,
                width: row.get::<_, i64>(1)? as u32,
                height: row.get::<_, i64>(2)? as u32,
                requested_size: row.get::<_, i64>(3)? as u32,
            })
        })
        .ok()
    }

    /// Saves a thumbnail to SQLite with optimized compression
    /// [WRITER]
    pub fn put(
        &self,
        path: &Path,
        modified: SystemTime,
        requested_size: u32,
        rgba_data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let id = Self::hash_path(path);
        let mod_time = modified
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // STEP 1: Process Image (Resize + Strip)
        if rgba_data.len() != (width * height * 4) as usize {
            return Err("Invalid RGBA data length".into());
        }

        let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
            ImageBuffer::from_raw(width, height, rgba_data.to_vec())
                .ok_or("Failed to create image buffer")?;
        let dynamic_img = DynamicImage::ImageRgba8(img);

        let resized = if width > 1024 || height > 1024 {
            dynamic_img.resize(1024, 1024, image::imageops::FilterType::Lanczos3)
        } else {
            dynamic_img
        };

        // STEP 2: Encode to WebP Lossy (preserve alpha channel for transparent images)
        let (final_width, final_height) = (resized.width(), resized.height());

        // Check if image has transparency (non-opaque alpha values)
        let has_alpha = resized.color().has_alpha();
        let webp_data = if has_alpha {
            // Preserve alpha channel for transparent images (PNG, SVG, etc.)
            let rgba_img = resized.to_rgba8();
            let encoder = webp::Encoder::from_rgba(&rgba_img, final_width, final_height);
            encoder.encode(85.0)
        } else {
            // Use RGB for opaque images (slightly smaller file size)
            let rgb_img = resized.to_rgb8();
            let encoder = webp::Encoder::from_rgb(&rgb_img, final_width, final_height);
            encoder.encode(85.0)
        };

        // STEP 3: Save to SQLite (Writer)
        let db = self.writer.lock().map_err(|_| "Database lock failed")?;
        let path_str = path.to_string_lossy().to_string();

        db.execute(
            "INSERT OR REPLACE INTO thumbnails
             (id, path, data, modified_at, created_at, width, height, requested_size)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                id,
                path_str,
                webp_data.to_vec(),
                mod_time,
                now,
                final_width as i64,
                final_height as i64,
                requested_size as i64
            ],
        )?;

        eprintln!(
            "[DB-PUT] OK id={} {}x{} req_size={} path={:?}",
            &id[..8],
            final_width,
            final_height,
            requested_size,
            path.file_name()
        );

        Ok(())
    }

    /// Sets a user preference
    /// [WRITER]
    pub fn set_preference(&self, key: &str, value: &str) {
        if let Ok(db) = self.writer.lock() {
            let _ = db.execute(
                "INSERT OR REPLACE INTO user_preferences (key, value) VALUES (?, ?)",
                params![key, value],
            );
        }
    }

    /// Gets a user preference
    /// [READER]
    pub fn get_preference(&self, key: &str) -> Option<String> {
        if let Ok(db) = self.reader.lock() {
            let mut stmt = db
                .prepare("SELECT value FROM user_preferences WHERE key = ?")
                .ok()?;
            stmt.query_row(params![key], |row| row.get(0)).ok()
        } else {
            None
        }
    }

    /// Gets covers (thumbnails) for multiple folders at once
    /// [READER]
    /// PERFORMANCE: Uses chunking to stay within SQLite's parameter limit (999)
    pub fn get_folder_covers(
        &self,
        folder_paths: &[PathBuf],
    ) -> std::collections::HashMap<PathBuf, PathBuf> {
        let mut results = std::collections::HashMap::new();
        if folder_paths.is_empty() {
            return results;
        }

        // SQLite parameter limit is 999, use 500 for safety margin
        const BATCH_SIZE: usize = 500;

        let db = match self.reader.lock() {
            Ok(db) => db,
            Err(_) => return results,
        };

        for chunk in folder_paths.chunks(BATCH_SIZE) {
            let placeholders: Vec<&str> = chunk.iter().map(|_| "?").collect();
            let query = format!(
                "SELECT folder_path, cover_path FROM folder_covers WHERE folder_path IN ({})",
                placeholders.join(",")
            );

            if let Ok(mut stmt) = db.prepare(&query) {
                let path_strs: Vec<String> = chunk
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();

                if let Ok(rows) =
                    stmt.query_map(rusqlite::params_from_iter(path_strs.iter()), |row| {
                        let f_path: String = row.get(0)?;
                        let c_path: String = row.get(1)?;
                        Ok((f_path, c_path))
                    })
                {
                    for row in rows.flatten() {
                        results.insert(PathBuf::from(row.0), PathBuf::from(row.1));
                    }
                }
            }
        }

        results
    }

    /// Obtém a capa (thumbnail) de uma pasta se já foi descoberta
    /// [READER]
    #[allow(dead_code)]
    pub fn get_folder_cover(&self, folder_path: &Path) -> Option<PathBuf> {
        let db = self.reader.lock().ok()?;
        let mut stmt = db
            .prepare_cached("SELECT cover_path FROM folder_covers WHERE folder_path = ?")
            .ok()?;
        let cover_path = stmt
            .query_row([folder_path.to_string_lossy()], |row| {
                let path_str: String = row.get(0)?;
                Ok(PathBuf::from(path_str))
            })
            .ok()?;

        // Validate that the cover path still exists before returning it
        // CRITICAL: Use fast_path_exists() instead of exists() to avoid blocking on OneDrive cloud-only files
        if crate::infrastructure::onedrive::fast_path_exists(&cover_path) {
            Some(cover_path)
        } else {
            None
        }
    }

    /// Salva a capa (thumbnail) descoberta para uma pasta
    /// [WRITER]
    pub fn set_folder_cover(&self, folder_path: &Path, cover_path: &Path) {
        if let Ok(db) = self.writer.lock() {
            let _ = db.execute(
                "INSERT OR REPLACE INTO folder_covers (folder_path, cover_path) VALUES (?, ?)",
                [folder_path.to_string_lossy(), cover_path.to_string_lossy()],
            );
        }
    }

    /// Remove a capa armazenada de uma pasta
    /// [WRITER]
    pub fn remove_folder_cover(&self, folder_path: &Path) {
        if let Ok(db) = self.writer.lock() {
            let _ = db.execute(
                "DELETE FROM folder_covers WHERE folder_path = ?",
                [folder_path.to_string_lossy()],
            );
        }
    }

    // ========== Folder Preview Cache (Shell Sandwich Icons) ==========

    /// Retrieves a cached folder preview (Shell sandwich icon) from SQLite.
    /// Returns decoded RGBA data ready for GPU upload.
    /// [READER]
    pub fn get_folder_preview_cache(
        &self,
        folder_path: &Path,
    ) -> Option<(Vec<u8>, u32, u32)> {
        let db = self.reader.lock().ok()?;
        let mut stmt = db
            .prepare_cached(
                "SELECT data, width, height FROM folder_previews WHERE folder_path = ?",
            )
            .ok()?;

        let folder_path_str = folder_path.to_string_lossy();
        let (webp_data, _db_width, _db_height): (Vec<u8>, u32, u32) = match stmt
            .query_row([&*folder_path_str], |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)? as u32,
                    row.get::<_, i64>(2)? as u32,
                ))
            }) {
            Ok(row) => row,
            Err(_) => return None,
        };

        // Decode WebP back to RGBA
        let decoder = webp::Decoder::new(&webp_data);
        let decoded = match decoder.decode() {
            Some(img) => img,
            None => {
                eprintln!(
                    "[FOLDER PREVIEW CACHE] WebP decode failed for {:?} ({} bytes)",
                    folder_path.file_name(),
                    webp_data.len()
                );
                return None;
            }
        };
        let rgba = decoded.to_image().to_rgba8();
        let (w, h) = (rgba.width(), rgba.height());
        Some((rgba.into_raw(), w, h))
    }

    /// Saves a folder preview (Shell sandwich icon) to SQLite, compressed as WebP.
    /// [WRITER]
    pub fn put_folder_preview_cache(
        &self,
        folder_path: &Path,
        rgba_data: &[u8],
        width: u32,
        height: u32,
    ) {
        if rgba_data.len() != (width * height * 4) as usize {
            return;
        }

        // Encode to WebP lossy (folder previews are small, ~256x256)
        let encoder = webp::Encoder::from_rgba(rgba_data, width, height);
        let webp_data = encoder.encode(85.0);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        if let Ok(db) = self.writer.lock() {
            let _ = db.execute(
                "INSERT OR REPLACE INTO folder_previews (folder_path, data, width, height, created_at)
                 VALUES (?, ?, ?, ?, ?)",
                params![
                    folder_path.to_string_lossy().to_string(),
                    webp_data.to_vec(),
                    width as i64,
                    height as i64,
                    now
                ],
            );
        }
    }

    /// Removes a cached folder preview.
    /// [WRITER]
    pub fn remove_folder_preview_cache(&self, folder_path: &Path) {
        if let Ok(db) = self.writer.lock() {
            let _ = db.execute(
                "DELETE FROM folder_previews WHERE folder_path = ?",
                [folder_path.to_string_lossy()],
            );
        }
    }

    /// Remove cache entries for a specific path (file or folder)
    /// [WRITER]
    pub fn remove_cache_for_path(&self, path: &Path) {
        let path_str = path.to_string_lossy().to_string();
        let path_str = path_str
            .strip_prefix(r"\\?\")
            .unwrap_or(&path_str)
            .to_string();

        if let Ok(db) = self.writer.lock() {
            let pattern = format!("{}\\%", path_str.trim_end_matches('\\'));

            // Remove entradas de thumbnails
            let _ = db.execute("DELETE FROM thumbnails WHERE path = ?", [&path_str]);
            let deleted = db
                .execute("DELETE FROM thumbnails WHERE path LIKE ?", [&pattern])
                .unwrap_or(0);

            // Remove folder cover entries
            let _ = db.execute(
                "DELETE FROM folder_covers WHERE folder_path = ?",
                [&path_str],
            );
            let _ = db.execute(
                "DELETE FROM folder_covers WHERE folder_path LIKE ?",
                [&pattern],
            );
            // Exact match: this file IS a folder cover
            let _ = db.execute(
                "DELETE FROM folder_covers WHERE cover_path = ?",
                [&path_str],
            );
            // Children match: covers inside a deleted folder
            let _ = db.execute(
                "DELETE FROM folder_covers WHERE cover_path LIKE ?",
                [&pattern],
            );

            // Remove folder preview cache entries
            let _ = db.execute(
                "DELETE FROM folder_previews WHERE folder_path = ?",
                [&path_str],
            );
            let _ = db.execute(
                "DELETE FROM folder_previews WHERE folder_path LIKE ?",
                [&pattern],
            );

            // Log cleanup (VACUUM is not called here to avoid UI thread blocking;
            // it runs during garbage_collect() which is called at controlled times)
            if deleted > 0 {
                eprintln!("[Cache] Cleaned {} entries for: {}", deleted, path_str);
            }
        }
    }

    fn path_exists_fast(path: &str) -> bool {
        crate::infrastructure::onedrive::fast_path_exists(Path::new(path))
    }

    /// Extract drive root (e.g., "X:\\") from a path string.
    fn extract_drive_root(path: &str) -> Option<String> {
        if path.len() >= 3
            && path.as_bytes()[0].is_ascii_alphabetic()
            && path.as_bytes()[1] == b':'
            && (path.as_bytes()[2] == b'\\' || path.as_bytes()[2] == b'/')
        {
            Some(format!("{}:\\", path.chars().next().unwrap()))
        } else {
            None
        }
    }

    /// Build a set of drive roots that are currently accessible.
    /// Entries on inaccessible drives (e.g., unmounted Cryptomator vaults)
    /// are skipped during GC to prevent deleting valid cached thumbnails.
    fn accessible_drives(
        paths: impl Iterator<Item = impl AsRef<str>>,
    ) -> std::collections::HashSet<String> {
        let mut checked: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
        let mut accessible = std::collections::HashSet::new();

        for path in paths {
            if let Some(root) = Self::extract_drive_root(path.as_ref()) {
                let is_ok = *checked
                    .entry(root.clone())
                    .or_insert_with(|| Self::path_exists_fast(&root));
                if is_ok {
                    accessible.insert(root);
                }
            }
        }
        accessible
    }

    /// Check if a path's drive is accessible (using a pre-built set).
    fn is_on_accessible_drive(path: &str, accessible: &std::collections::HashSet<String>) -> bool {
        match Self::extract_drive_root(path) {
            Some(root) => accessible.contains(&root),
            None => true, // Network paths, etc. — always check
        }
    }

    fn execute_batch_delete(
        db: &Connection,
        table: &str,
        key_col: &str,
        items: &[String],
    ) -> usize {
        let mut count = 0;
        const BATCH_SIZE: usize = 500;

        for chunk in items.chunks(BATCH_SIZE) {
            let placeholders = std::iter::repeat_n("?", chunk.len())
                .collect::<Vec<_>>()
                .join(",");

            let sql = format!(
                "DELETE FROM {} WHERE {} IN ({})",
                table, key_col, placeholders
            );

            match db.execute(&sql, rusqlite::params_from_iter(chunk.iter())) {
                Ok(c) => count += c,
                Err(e) => eprintln!("[GC] Failed to delete batch from {}: {:?}", table, e),
            }
        }

        count
    }

    /// Incremental GC pass: scans only a bounded sample to keep I/O low.
    /// Intended to run periodically in background idle windows.
    pub fn garbage_collect_incremental(&self, max_candidates: usize) -> usize {
        let limit = max_candidates.max(1) as i64;

        let sampled_entries: Vec<(String, String)>;
        let sampled_folders: Vec<String>;
        let sampled_folder_previews: Vec<String>;

        {
            let db = match self.reader.lock() {
                Ok(db) => db,
                Err(_) => {
                    eprintln!("[GC] Incremental pass skipped: reader lock failed");
                    return 0;
                }
            };

            sampled_entries = db
                .prepare(
                    "SELECT id, path FROM thumbnails WHERE path IS NOT NULL ORDER BY RANDOM() LIMIT ?1",
                )
                .and_then(|mut stmt| {
                    stmt.query_map(params![limit], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .map(|rows| rows.flatten().collect())
                })
                .unwrap_or_default();

            sampled_folders = db
                .prepare("SELECT folder_path FROM folder_covers ORDER BY RANDOM() LIMIT ?1")
                .and_then(|mut stmt| {
                    stmt.query_map(params![limit], |row| row.get::<_, String>(0))
                        .map(|rows| rows.flatten().collect())
                })
                .unwrap_or_default();

            sampled_folder_previews = db
                .prepare("SELECT folder_path FROM folder_previews ORDER BY RANDOM() LIMIT ?1")
                .and_then(|mut stmt| {
                    stmt.query_map(params![limit], |row| row.get::<_, String>(0))
                        .map(|rows| rows.flatten().collect())
                })
                .unwrap_or_default();
        }

        if sampled_entries.is_empty()
            && sampled_folders.is_empty()
            && sampled_folder_previews.is_empty()
        {
            return 0;
        }

        // CRITICAL: Determine which drives are currently accessible.
        // Skip orphan-checking for files on inaccessible drives (e.g., unmounted
        // Cryptomator vaults) to prevent deleting valid cached thumbnails.
        let all_paths = sampled_entries
            .iter()
            .map(|(_, p)| p.as_str())
            .chain(sampled_folders.iter().map(|p| p.as_str()))
            .chain(sampled_folder_previews.iter().map(|p| p.as_str()));
        let accessible = Self::accessible_drives(all_paths);

        let orphan_thumbs: Vec<String> = sampled_entries
            .into_iter()
            .filter(|(_, path)| {
                Self::is_on_accessible_drive(path, &accessible) && !Self::path_exists_fast(path)
            })
            .map(|(id, _)| id)
            .collect();

        let orphan_folders: Vec<String> = sampled_folders
            .into_iter()
            .filter(|path| {
                Self::is_on_accessible_drive(path, &accessible) && !Self::path_exists_fast(path)
            })
            .collect();

        let orphan_folder_previews: Vec<String> = sampled_folder_previews
            .into_iter()
            .filter(|path| {
                Self::is_on_accessible_drive(path, &accessible) && !Self::path_exists_fast(path)
            })
            .collect();

        if orphan_thumbs.is_empty()
            && orphan_folders.is_empty()
            && orphan_folder_previews.is_empty()
        {
            return 0;
        }

        let mut removed = 0;
        if let Ok(db) = self.writer.lock() {
            let _ = db.execute("BEGIN TRANSACTION", []);
            if !orphan_thumbs.is_empty() {
                removed += Self::execute_batch_delete(&db, "thumbnails", "id", &orphan_thumbs);
            }
            if !orphan_folders.is_empty() {
                removed += Self::execute_batch_delete(
                    &db,
                    "folder_covers",
                    "folder_path",
                    &orphan_folders,
                );
            }
            if !orphan_folder_previews.is_empty() {
                removed += Self::execute_batch_delete(
                    &db,
                    "folder_previews",
                    "folder_path",
                    &orphan_folder_previews,
                );
            }
            let _ = db.execute("COMMIT", []);
        }

        if removed > 0 {
            eprintln!("[GC] Incremental pass removed {} entries", removed);
        }
        removed
    }

    /// Runs VACUUM explicitly (heavy operation, call rarely).
    pub fn run_vacuum(&self) -> bool {
        match self.writer.lock() {
            Ok(db) => db.execute("VACUUM", []).is_ok(),
            Err(_) => false,
        }
    }

    /// Full GC: scans all cache rows. Use sparingly.
    pub fn garbage_collect(&self) -> usize {
        eprintln!("[GC] Starting full garbage collection...");

        let all_entries: Vec<(String, String)>;
        let all_folders: Vec<String>;
        let all_folder_previews: Vec<String>;

        {
            let db = match self.reader.lock() {
                Ok(db) => db,
                Err(_) => {
                    eprintln!("[GC] Failed to acquire database lock!");
                    return 0;
                }
            };

            all_entries = db
                .prepare("SELECT id, path FROM thumbnails WHERE path IS NOT NULL")
                .and_then(|mut stmt| {
                    stmt.query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .map(|rows| rows.flatten().collect())
                })
                .unwrap_or_default();

            all_folders = db
                .prepare("SELECT folder_path FROM folder_covers")
                .and_then(|mut stmt| {
                    stmt.query_map([], |row| row.get::<_, String>(0))
                        .map(|rows| rows.flatten().collect())
                })
                .unwrap_or_default();

            all_folder_previews = db
                .prepare("SELECT folder_path FROM folder_previews")
                .and_then(|mut stmt| {
                    stmt.query_map([], |row| row.get::<_, String>(0))
                        .map(|rows| rows.flatten().collect())
                })
                .unwrap_or_default();
        }

        // CRITICAL: Skip orphan-checking for files on inaccessible drives
        // (e.g., unmounted Cryptomator vaults) to prevent mass-deleting valid cache.
        let all_paths = all_entries
            .iter()
            .map(|(_, p)| p.as_str())
            .chain(all_folders.iter().map(|p| p.as_str()))
            .chain(all_folder_previews.iter().map(|p| p.as_str()));
        let accessible = Self::accessible_drives(all_paths);

        let orphan_thumbs: Vec<String> = all_entries
            .into_iter()
            .filter(|(_, path)| {
                Self::is_on_accessible_drive(path, &accessible) && !Self::path_exists_fast(path)
            })
            .map(|(id, _)| id)
            .collect();

        let orphan_folders: Vec<String> = all_folders
            .into_iter()
            .filter(|path| {
                Self::is_on_accessible_drive(path, &accessible) && !Self::path_exists_fast(path)
            })
            .collect();

        let orphan_folder_previews: Vec<String> = all_folder_previews
            .into_iter()
            .filter(|path| {
                Self::is_on_accessible_drive(path, &accessible) && !Self::path_exists_fast(path)
            })
            .collect();

        if orphan_thumbs.is_empty()
            && orphan_folders.is_empty()
            && orphan_folder_previews.is_empty()
        {
            eprintln!("[GC] No orphans found, skipping cleanup");
            return 0;
        }

        let mut removed = 0;
        if let Ok(db) = self.writer.lock() {
            let _ = db.execute("BEGIN TRANSACTION", []);
            if !orphan_thumbs.is_empty() {
                removed += Self::execute_batch_delete(&db, "thumbnails", "id", &orphan_thumbs);
            }
            if !orphan_folders.is_empty() {
                removed += Self::execute_batch_delete(
                    &db,
                    "folder_covers",
                    "folder_path",
                    &orphan_folders,
                );
            }
            if !orphan_folder_previews.is_empty() {
                removed += Self::execute_batch_delete(
                    &db,
                    "folder_previews",
                    "folder_path",
                    &orphan_folder_previews,
                );
            }
            let _ = db.execute("COMMIT", []);
        }

        if removed > 0 {
            eprintln!(
                "[GC] Full GC removed {} entries (VACUUM not automatic)",
                removed
            );
        }
        removed
    }
}
