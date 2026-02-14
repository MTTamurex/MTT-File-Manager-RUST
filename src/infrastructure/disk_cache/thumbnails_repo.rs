use super::{ThumbnailCacheEntry, ThumbnailDiskCache};
use image::{DynamicImage, ImageBuffer, Rgba};
use rusqlite::params;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

impl ThumbnailDiskCache {
    /// Generates a stable, collision-resistant hash for a file path.
    /// Uses blake3 (128-bit) instead of DefaultHasher (64-bit, unstable across Rust versions).
    fn hash_path(path: &Path) -> String {
        let hash = blake3::hash(path.as_os_str().as_encoded_bytes());
        // Use first 128 bits (32 hex chars) - collision probability ~1 in 2^64 at 10B entries
        hash.to_hex()[..32].to_string()
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
            log::trace!(
                "[DB-MISS] get_latest: id={} path={:?} -> 0 rows in DB",
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
        let expected_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4));
        if expected_len.map_or(true, |n| rgba_data.len() != n) {
            return Err("Invalid RGBA data length".into());
        }

        let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
            ImageBuffer::from_raw(width, height, rgba_data.to_vec())
                .ok_or("Failed to create image buffer")?;
        let dynamic_img = DynamicImage::ImageRgba8(img);

        let resized = if width > 1024 || height > 1024 {
            dynamic_img.resize(1024, 1024, image::imageops::FilterType::CatmullRom)
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

        log::trace!(
            "[DB-PUT] OK id={} {}x{} req_size={} path={:?}",
            &id[..8],
            final_width,
            final_height,
            requested_size,
            path.file_name()
        );

        Ok(())
    }
}
