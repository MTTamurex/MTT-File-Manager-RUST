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

/// Manages persistent thumbnail storage in SQLite
pub struct ThumbnailDiskCache {
    writer: Arc<Mutex<Connection>>, // For put, set_*, garbage_collect (DELETE)
    reader: Arc<Mutex<Connection>>, // For get, get_*, check existence
    #[allow(dead_code)]
    cache_dir: PathBuf,
}

impl ThumbnailDiskCache {
    /// Creates a new disk cache at the specified directory
    pub fn new(cache_dir: PathBuf) -> Self {
        // Ensure directory exists
        if !cache_dir.exists() {
            let _ = fs::create_dir_all(&cache_dir);
        }

        // Clean up legacy files if they exist (Migration)
        Self::cleanup_legacy(&cache_dir);

        let db_path = cache_dir.join("thumbnails.db");

        // 1. Open WRITER connection (Primary)
        let writer_conn = match Connection::open(&db_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "[Cache] Failed to open database: {:?}. Using in-memory fallback.",
                    e
                );
                // Fallback to in-memory if disk database fails
                match Connection::open_in_memory() {
                    Ok(c) => c,
                    Err(fatal_e) => {
                        panic!(
                            "[FATAL] Cannot create even an in-memory database: {:?}",
                            fatal_e
                        );
                    }
                }
            }
        };

        // Performance Tuning: Use WAL mode for better concurrency (readers don't block writers)
        // and NORMAL synchronous for faster writes (safe in WAL mode).
        let _ = writer_conn.execute("PRAGMA journal_mode = WAL", []).ok();
        let _ = writer_conn.execute("PRAGMA synchronous = NORMAL", []).ok();

        // 2. Open READER connection (Secondary)
        // In WAL mode, this can read while writer is busy
        let reader_conn = match Connection::open(&db_path) {
            Ok(c) => c,
            Err(_) => {
                // Return a clone of writer if secondary fails (unlikely)
                // or open in memory if original was in memory?
                // Simplest fallback: Just open again.
                // If main failed, we handled it. If main succeeded, this should too.
                Connection::open(&db_path).unwrap_or_else(|_| Connection::open_in_memory().unwrap())
            }
        };
        let _ = reader_conn.execute("PRAGMA synchronous = NORMAL", []).ok();

        // 3. Schema Migrations (Run on Writer)
        Self::run_migrations(&writer_conn);

        Self {
            writer: Arc::new(Mutex::new(writer_conn)),
            reader: Arc::new(Mutex::new(reader_conn)),
            cache_dir,
        }
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

    /// Tries to retrieve a thumbnail from SQLite with dimensions
    /// [READER] concurrency friendly
    pub fn get(&self, path: &Path, modified: SystemTime) -> Option<(Vec<u8>, u32, u32)> {
        let id = Self::hash_path(path);
        let mod_time = modified
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let db = self.reader.lock().ok()?;
        let mut stmt = db
            .prepare_cached(
                "SELECT data, width, height FROM thumbnails WHERE id = ? AND modified_at = ?",
            )
            .ok()?;

        stmt.query_row(params![id, mod_time], |row| {
            let data: Vec<u8> = row.get(0)?;
            let width_i64: i64 = row.get(1)?;
            let height_i64: i64 = row.get(2)?;
            Ok((data, width_i64 as u32, height_i64 as u32))
        })
        .ok()
    }

    /// Saves a thumbnail to SQLite with optimized compression
    /// [WRITER]
    pub fn put(
        &self,
        path: &Path,
        modified: SystemTime,
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
            "INSERT OR REPLACE INTO thumbnails (id, path, data, modified_at, created_at, width, height) VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![id, path_str, webp_data.to_vec(), mod_time, now, final_width as i64, final_height as i64],
        )?;

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
        }

        if sampled_entries.is_empty() && sampled_folders.is_empty() {
            return 0;
        }

        let orphan_thumbs: Vec<String> = sampled_entries
            .into_iter()
            .filter(|(_, path)| !Self::path_exists_fast(path))
            .map(|(id, _)| id)
            .collect();

        let orphan_folders: Vec<String> = sampled_folders
            .into_iter()
            .filter(|path| !Self::path_exists_fast(path))
            .collect();

        if orphan_thumbs.is_empty() && orphan_folders.is_empty() {
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
        }

        let orphan_thumbs: Vec<String> = all_entries
            .into_iter()
            .filter(|(_, path)| !Self::path_exists_fast(path))
            .map(|(id, _)| id)
            .collect();

        let orphan_folders: Vec<String> = all_folders
            .into_iter()
            .filter(|path| !Self::path_exists_fast(path))
            .collect();

        if orphan_thumbs.is_empty() && orphan_folders.is_empty() {
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
