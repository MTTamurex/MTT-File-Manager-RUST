//! Persistent SQLite cache for thumbnails
//! Follows .cursorrules: I/O in worker threads, RAII for resources

use std::path::{Path, PathBuf};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use image::{DynamicImage, ImageBuffer, Rgba};
use rusqlite::{params, Connection};

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
        let conn = Connection::open(db_path).expect("Failed to open thumbnail database");

        // Performance Tuning: WAL mode and Normal sync
        let _ = conn.execute("PRAGMA journal_mode = WAL", []).ok();
        let _ = conn.execute("PRAGMA synchronous = NORMAL", []).ok();

        // Create table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS thumbnails (
                id TEXT PRIMARY KEY,
                data BLOB,
                modified_at INTEGER,
                created_at INTEGER
            )",
            [],
        ).expect("Failed to create thumbnails table");

        Self { 
            db: Arc::new(Mutex::new(conn)),
            cache_dir 
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
        let mut stmt = db.prepare_cached("SELECT data FROM thumbnails WHERE id = ? AND modified_at = ?").ok()?;
        
        stmt.query_row(params![id, mod_time], |row| row.get(0)).ok()
    }

    /// Saves a thumbnail to SQLite with optimized compression
    pub fn put(&self, path: &Path, modified: SystemTime, rgba_data: &[u8], width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
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
        
        let img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_raw(width, height, rgba_data.to_vec())
            .ok_or("Failed to create image buffer")?;
        let dynamic_img = DynamicImage::ImageRgba8(img);
        
        // Resize to max 200px (longest side) for disk efficiency
        let resized = dynamic_img.resize(200, 200, image::imageops::FilterType::Lanczos3);
        
        // STEP 2: Encode to WebP (Quality 60)
        let mut webp_data = Vec::new();
        {
            let mut cursor = std::io::Cursor::new(&mut webp_data);
            use image::codecs::webp::WebPEncoder;
            
            // Revertendo para lossless para garantir a compilação
            // O redimensionamento para 200px já reduzirá drasticamente o espaço em disco
            let encoder = WebPEncoder::new_lossless(&mut cursor);
            resized.write_with_encoder(encoder)?;
        }

        // STEP 3: Save to SQLite
        let db = self.db.lock().map_err(|_| "Database lock failed")?;
        db.execute(
            "INSERT OR REPLACE INTO thumbnails (id, data, modified_at, created_at) VALUES (?, ?, ?, ?)",
            params![id, webp_data, mod_time, now],
        )?;

        Ok(())
    }
}
