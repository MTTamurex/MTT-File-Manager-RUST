use super::{ThumbnailCacheEntry, ThumbnailDiskCache};
use image::{DynamicImage, ImageBuffer, Rgba};
use rusqlite::params;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const GET_LATEST_BATCH_CHUNK_SIZE: usize = 500;

impl ThumbnailDiskCache {
    /// Generates a stable, collision-resistant hash for a file path.
    /// Uses blake3 (128-bit) instead of DefaultHasher (64-bit, unstable across Rust versions).
    pub(super) fn hash_path(path: &Path) -> String {
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
                "SELECT data, width, height, requested_size, modified_at
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
                modified_at: row.get::<_, i64>(4)? as u64,
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

        let mut stmt = db
            .prepare_cached(
                "SELECT data, width, height, requested_size, modified_at
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
                modified_at: row.get::<_, i64>(4)? as u64,
            })
        })
        .ok()
    }

    /// Retrieves the latest thumbnail entries for paths, preserving input order.
    /// Duplicate input paths produce duplicate output entries.
    /// [READER] concurrency friendly
    pub fn get_latest_batch(&self, paths: &[PathBuf]) -> Vec<Option<ThumbnailCacheEntry>> {
        if paths.is_empty() {
            return Vec::new();
        }

        let ids: Vec<String> = paths.iter().map(|path| Self::hash_path(path)).collect();
        let db = match self.reader.lock() {
            Ok(db) => db,
            Err(_) => return vec![None; paths.len()],
        };

        let mut entries_by_id: HashMap<String, ThumbnailCacheEntry> =
            HashMap::with_capacity(ids.len());

        for chunk in ids.chunks(GET_LATEST_BATCH_CHUNK_SIZE) {
            if chunk.is_empty() {
                continue;
            }

            let placeholders = std::iter::repeat_n("?", chunk.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT id, data, width, height, requested_size, modified_at
                 FROM thumbnails
                 WHERE id IN ({})",
                placeholders
            );

            let Ok(mut stmt) = db.prepare(&sql) else {
                continue;
            };
            let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    ThumbnailCacheEntry {
                        data: row.get(1)?,
                        width: row.get::<_, i64>(2)? as u32,
                        height: row.get::<_, i64>(3)? as u32,
                        requested_size: row.get::<_, i64>(4)? as u32,
                        modified_at: row.get::<_, i64>(5)? as u64,
                    },
                ))
            }) else {
                continue;
            };

            for row in rows.flatten() {
                let (id, entry) = row;
                entries_by_id.insert(id, entry);
            }
        }

        ids.iter()
            .map(|id| entries_by_id.get(id).cloned())
            .collect()
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
        if expected_len != Some(rgba_data.len()) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::tempdir;

    fn rgba(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            out.extend_from_slice(&color);
        }
        out
    }

    #[test]
    fn get_latest_batch_preserves_order_hits_misses_and_duplicates() {
        let dir = tempdir().expect("create temp dir");
        let cache =
            ThumbnailDiskCache::new(dir.path().to_path_buf()).expect("create thumbnail cache");
        let modified = UNIX_EPOCH + Duration::from_secs(10);

        let path_a = dir.path().join("a.jpg");
        let path_b = dir.path().join("b.jpg");
        let missing = dir.path().join("missing.jpg");

        cache
            .put(&path_a, modified, 128, &rgba(2, 2, [255, 0, 0, 255]), 2, 2)
            .expect("put first thumbnail");
        cache
            .put(&path_b, modified, 256, &rgba(3, 1, [0, 255, 0, 255]), 3, 1)
            .expect("put second thumbnail");

        let results = cache.get_latest_batch(&[
            missing.clone(),
            path_b.clone(),
            path_a.clone(),
            path_b.clone(),
        ]);

        assert_eq!(results.len(), 4);
        assert!(results[0].is_none());
        assert_eq!(
            results[1]
                .as_ref()
                .map(|entry| (entry.width, entry.height, entry.requested_size)),
            Some((3, 1, 256))
        );
        assert_eq!(
            results[2]
                .as_ref()
                .map(|entry| (entry.width, entry.height, entry.requested_size)),
            Some((2, 2, 128))
        );
        assert_eq!(
            results[3]
                .as_ref()
                .map(|entry| (entry.width, entry.height, entry.requested_size)),
            Some((3, 1, 256))
        );
        assert!(results[1].as_ref().is_some_and(|entry| !entry.data.is_empty()));
    }
}
