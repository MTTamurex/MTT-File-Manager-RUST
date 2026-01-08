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
    db: Arc<Mutex<Connection>>,
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
        let conn = match Connection::open(&db_path) {
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
                        // This really shouldn't happen, but if it does, we must panic as we need a DB connection
                        panic!(
                            "[FATAL] Cannot create even an in-memory database: {:?}",
                            fatal_e
                        );
                    }
                }
            }
        };

        // Performance Tuning: Use DELETE mode for immediate sync (WAL was causing issues)
        let _ = conn.execute("PRAGMA journal_mode = DELETE", []).ok();
        let _ = conn.execute("PRAGMA synchronous = FULL", []).ok();

        // Create table (with path for GC)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS thumbnails (
                id TEXT PRIMARY KEY,
                path TEXT,
                data BLOB,
                modified_at INTEGER,
                created_at INTEGER
            )",
            [],
        )
        .unwrap_or_else(|e| {
            eprintln!(
                "[Cache] Warning: Failed to create thumbnails table: {:?}",
                e
            );
            0 // continue
        });

        // Migration: Add path column if missing (for existing DBs)
        let _ = conn.execute("ALTER TABLE thumbnails ADD COLUMN path TEXT", []);

        // Create preferences table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS user_preferences (
                key TEXT PRIMARY KEY,
                value TEXT
            )",
            [],
        )
        .unwrap_or_else(|e| {
            eprintln!(
                "[Cache] Warning: Failed to create preferences table: {:?}",
                e
            );
            0 // continue
        });

        // Create folder covers table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS folder_covers (
                folder_path TEXT PRIMARY KEY,
                cover_path TEXT
            )",
            [],
        )
        .unwrap_or_else(|e| {
            eprintln!(
                "[Cache] Warning: Failed to create folder covers table: {:?}",
                e
            );
            0 // continue
        });

        Self {
            db: Arc::new(Mutex::new(conn)),
            cache_dir,
        }
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

    /// Tries to retrieve a thumbnail from SQLite
    pub fn get(&self, path: &Path, modified: SystemTime) -> Option<Vec<u8>> {
        let id = Self::hash_path(path);
        let mod_time = modified
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let db = self.db.lock().ok()?;
        let mut stmt = db
            .prepare_cached("SELECT data FROM thumbnails WHERE id = ? AND modified_at = ?")
            .ok()?;

        stmt.query_row(params![id, mod_time], |row| row.get(0)).ok()
    }

    /// Saves a thumbnail to SQLite with optimized compression
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
        // Ensure rgba_data has correct size before creating buffer
        if rgba_data.len() != (width * height * 4) as usize {
            return Err("Invalid RGBA data length".into());
        }

        let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
            ImageBuffer::from_raw(width, height, rgba_data.to_vec())
                .ok_or("Failed to create image buffer")?;
        let dynamic_img = DynamicImage::ImageRgba8(img);

        // Adaptive resize: only downscale if larger than 1024px, never upscale
        // This preserves high-quality thumbnails for the preview panel
        let resized = if width > 1024 || height > 1024 {
            dynamic_img.resize(1024, 1024, image::imageops::FilterType::Lanczos3)
        } else {
            dynamic_img // Keep original size
        };

        // STEP 2: Encode to WebP Lossy (Quality 60 - optimized for HiDPI)
        // Convert to RGB8 for webp crate (it doesn't support RGBA directly)
        let rgb_img = resized.to_rgb8();
        let (final_width, final_height) = (rgb_img.width(), rgb_img.height());

        // Use webp crate for lossy compression with quality control
        let encoder = webp::Encoder::from_rgb(&rgb_img, final_width, final_height);
        let webp_data = encoder.encode(60.0); // Quality 60 (0-100 scale)

        // STEP 3: Save to SQLite (with path for GC)
        let db = self.db.lock().map_err(|_| "Database lock failed")?;
        let path_str = path.to_string_lossy().to_string();

        // DEBUG: Log first few saves
        static SAVE_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let count = SAVE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if count < 3 {
            eprintln!("[PUT] Saving thumbnail for: {}", path_str);
        }

        db.execute(
            "INSERT OR REPLACE INTO thumbnails (id, path, data, modified_at, created_at) VALUES (?, ?, ?, ?, ?)",
            params![id, path_str, webp_data.to_vec(), mod_time, now],
        )?;

        Ok(())
    }

    /// Sets a user preference
    pub fn set_preference(&self, key: &str, value: &str) {
        if let Ok(db) = self.db.lock() {
            let _ = db.execute(
                "INSERT OR REPLACE INTO user_preferences (key, value) VALUES (?, ?)",
                params![key, value],
            );
        }
    }

    /// Gets a user preference
    pub fn get_preference(&self, key: &str) -> Option<String> {
        if let Ok(db) = self.db.lock() {
            let mut stmt = db
                .prepare("SELECT value FROM user_preferences WHERE key = ?")
                .ok()?;
            stmt.query_row(params![key], |row| row.get(0)).ok()
        } else {
            None
        }
    }

    /// Obtém a capa (thumbnail) de uma pasta se já foi descoberta
    pub fn get_folder_cover(&self, folder_path: &Path) -> Option<PathBuf> {
        let db = self.db.lock().ok()?;
        let mut stmt = db
            .prepare_cached("SELECT cover_path FROM folder_covers WHERE folder_path = ?")
            .ok()?;
        let cover_path = stmt.query_row([folder_path.to_string_lossy()], |row| {
            let path_str: String = row.get(0)?;
            Ok(PathBuf::from(path_str))
        })
        .ok()?;
        
        // Validate that the cover path still exists before returning it
        if cover_path.exists() {
            Some(cover_path)
        } else {
            // Cover file no longer exists - return None to trigger re-scan
            None
        }
    }

    /// Salva a capa (thumbnail) descoberta para uma pasta
    pub fn set_folder_cover(&self, folder_path: &Path, cover_path: &Path) {
        if let Ok(db) = self.db.lock() {
            let _ = db.execute(
                "INSERT OR REPLACE INTO folder_covers (folder_path, cover_path) VALUES (?, ?)",
                [folder_path.to_string_lossy(), cover_path.to_string_lossy()],
            );
        }
    }

    /// Remove cache entries for a specific path (file or folder)
    /// If the path is a folder, removes all entries that start with that path
    pub fn remove_cache_for_path(&self, path: &Path) {
        // Normaliza o path removendo o prefixo \\?\ do Windows
        let path_str = path.to_string_lossy().to_string();
        let path_str = path_str
            .strip_prefix(r"\\?\")
            .unwrap_or(&path_str)
            .to_string();

        if let Ok(db) = self.db.lock() {
            // Pattern: C:\folder\* (precisa adicionar barra antes de %)
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
            let _ = db.execute(
                "DELETE FROM folder_covers WHERE cover_path LIKE ?",
                [&pattern],
            );

            // Se deletou algo, roda VACUUM para reduzir tamanho do arquivo
            if deleted > 0 {
                let _ = db.execute("VACUUM", []);
                eprintln!("[Cache] Cleaned {} entries for: {}", deleted, path_str);
            }
        }
    }

    /// Garbage Collector: Remove entradas de arquivos que não existem mais
    /// OTIMIZADO: Libera o lock durante verificações de arquivo (I/O lento)
    /// Roda em background na inicialização para não bloquear a UI
    pub fn garbage_collect(&self) -> usize {
        eprintln!("[GC] Starting garbage collection...");

        let mut removed = 0;

        // FASE 1: Lê todos os paths (lock curto - apenas leitura do banco)
        let all_entries: Vec<(String, String)>;
        let all_folders: Vec<String>;

        {
            let db = match self.db.lock() {
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
        // ^^^ Lock liberado aqui!

        eprintln!(
            "[GC] Loaded {} thumbnails, {} folder covers to check",
            all_entries.len(),
            all_folders.len()
        );

        // FASE 2: Verifica existência de arquivos (SEM lock - I/O puro)
        // Esta é a parte lenta, mas não bloqueia o banco
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

        // FASE 3: Remove órfãos usando BATCH TRANSACTION (1 commit ao invés de N)
        if !orphan_thumbs.is_empty() || !orphan_folders.is_empty() {
            if let Ok(db) = self.db.lock() {
                // Inicia transação única - todas as deleções acontecem na memória
                let _ = db.execute("BEGIN TRANSACTION", []);

                // Remove thumbnails órfãos
                for id in &orphan_thumbs {
                    if db
                        .execute("DELETE FROM thumbnails WHERE id = ?", [id])
                        .is_ok()
                    {
                        removed += 1;
                    }
                }

                // Remove folder_covers órfãos
                for folder in &orphan_folders {
                    if db
                        .execute("DELETE FROM folder_covers WHERE folder_path = ?", [folder])
                        .is_ok()
                    {
                        removed += 1;
                    }
                }

                // Commit único - grava tudo no disco de uma vez
                let _ = db.execute("COMMIT", []);

                // VACUUM apenas se removeu algo
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
