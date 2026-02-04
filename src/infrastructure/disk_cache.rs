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

        // STEP 2: Encode to WebP Lossy
        let rgb_img = resized.to_rgb8();
        let (final_width, final_height) = (rgb_img.width(), rgb_img.height());
        let encoder = webp::Encoder::from_rgb(&rgb_img, final_width, final_height);
        let webp_data = encoder.encode(85.0);

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
    pub fn get_folder_covers(
        &self,
        folder_paths: &[PathBuf],
    ) -> std::collections::HashMap<PathBuf, PathBuf> {
        let mut results = std::collections::HashMap::new();
        if folder_paths.is_empty() {
            return results;
        }

        let mut raw_results = Vec::new();

        {
            let db = match self.reader.lock() {
                Ok(db) => db,
                Err(_) => return results,
            };

            // SQLite parameter limit is usually 999, so 250 (batch size) is safe
            let placeholders: Vec<&str> = folder_paths.iter().map(|_| "?").collect();
            let query = format!(
                "SELECT folder_path, cover_path FROM folder_covers WHERE folder_path IN ({})",
                placeholders.join(",")
            );

            if let Ok(mut stmt) = db.prepare(&query) {
                let path_strs: Vec<String> = folder_paths
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
                        raw_results.push((row.0, row.1));
                    }
                }
            };
        }

        for (f_path, c_path) in raw_results {
            results.insert(PathBuf::from(f_path), PathBuf::from(c_path));
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
        if cover_path.exists() {
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

            // Se deletou algo, roda VACUUM
            if deleted > 0 {
                let _ = db.execute("VACUUM", []);
                eprintln!("[Cache] Cleaned {} entries for: {}", deleted, path_str);
            }
        }
    }

    /// Garbage Collector: Remove entradas de arquivos que não existem mais
    /// OTIMIZADO: Libera o lock durante verificações de arquivo (I/O lento)
    pub fn garbage_collect(&self) -> usize {
        eprintln!("[GC] Starting garbage collection...");

        let mut removed = 0;

        // FASE 1: Lê todos os paths ([READER] lock curto)
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

            // Coleta thumbnails
            all_entries = db
                .prepare("SELECT id, path FROM thumbnails WHERE path IS NOT NULL")
                .and_then(|mut stmt| {
                    stmt.query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .map(|rows| rows.flatten().collect())
                })
                .unwrap_or_default();

            // Coleta folder_covers
            all_folders = db
                .prepare("SELECT folder_path FROM folder_covers")
                .and_then(|mut stmt| {
                    stmt.query_map([], |row| row.get::<_, String>(0))
                        .map(|rows| rows.flatten().collect())
                })
                .unwrap_or_default();
        }

        eprintln!(
            "[GC] Loaded {} thumbnails, {} folder covers to check",
            all_entries.len(),
            all_folders.len()
        );

        // FASE 2: Verifica existência de arquivos (SEM lock - I/O puro)
        let orphan_thumbs: Vec<String> = all_entries
            .into_iter()
            .filter(|(_, path)| !Path::new(path).exists())
            .map(|(id, _)| id)
            .collect();

        let orphan_folders: Vec<String> = all_folders
            .into_iter()
            .filter(|path| !Path::new(path).exists())
            .collect();

        eprintln!(
            "[GC] Found {} orphan thumbnails, {} orphan folders",
            orphan_thumbs.len(),
            orphan_folders.len()
        );

        // FASE 3: Remove órfãos usando BATCH DELETE ([WRITER] lock)
        if !orphan_thumbs.is_empty() || !orphan_folders.is_empty() {
            if let Ok(db) = self.writer.lock() {
                // Inicia transação
                let _ = db.execute("BEGIN TRANSACTION", []);

                // Helper local
                let execute_batch_delete =
                    |table: &str, key_col: &str, items: &[String]| -> usize {
                        let mut count = 0;
                        const BATCH_SIZE: usize = 500;

                        for chunk in items.chunks(BATCH_SIZE) {
                            let placeholders = std::iter::repeat("?")
                                .take(chunk.len())
                                .collect::<Vec<_>>()
                                .join(",");

                            let sql = format!(
                                "DELETE FROM {} WHERE {} IN ({})",
                                table, key_col, placeholders
                            );

                            match db.execute(&sql, rusqlite::params_from_iter(chunk.iter())) {
                                Ok(c) => count += c,
                                Err(e) => {
                                    eprintln!("[GC] Failed to delete batch from {}: {:?}", table, e)
                                }
                            }
                        }
                        count
                    };

                if !orphan_thumbs.is_empty() {
                    removed += execute_batch_delete("thumbnails", "id", &orphan_thumbs);
                }

                if !orphan_folders.is_empty() {
                    removed +=
                        execute_batch_delete("folder_covers", "folder_path", &orphan_folders);
                }

                let _ = db.execute("COMMIT", []);

                if removed > 0 {
                    eprintln!("[GC] Removed {} entries, running VACUUM...", removed);
                    let _ = db.execute("VACUUM", []);
                }
            }
        } else {
            eprintln!("[GC] No orphans found, skipping cleanup");
        }

        removed
    }
}
