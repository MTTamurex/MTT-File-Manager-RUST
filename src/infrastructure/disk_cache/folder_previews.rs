use super::ThumbnailDiskCache;
use rusqlite::params;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

impl ThumbnailDiskCache {
    /// Retrieves a cached folder preview (Shell sandwich icon) from SQLite.
    /// Returns decoded RGBA data ready for GPU upload, plus the cache timestamp.
    /// The `created_at` (Unix seconds) allows callers to detect stale entries
    /// by comparing against the folder's last-write time.
    /// [READER]
    pub fn get_folder_preview_cache(&self, folder_path: &Path) -> Option<(Vec<u8>, u32, u32, i64)> {
        let db = self.reader.lock().ok()?;
        let mut stmt = db
            .prepare_cached(
                "SELECT data, width, height, created_at FROM folder_previews WHERE folder_path = ?",
            )
            .ok()?;

        let folder_path_str = folder_path.to_string_lossy();
        let (webp_data, _db_width, _db_height, created_at): (Vec<u8>, u32, u32, i64) = match stmt
            .query_row([&*folder_path_str], |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)? as u32,
                    row.get::<_, i64>(2)? as u32,
                    row.get::<_, i64>(3)?,
                ))
            }) {
            Ok(row) => row,
            Err(_) => return None,
        };

        // Validate WebP container header before passing to decoder.
        // This catches obvious corruption/tampering before the codec processes
        // the data, reducing attack surface against WebP decoder vulnerabilities.
        if webp_data.len() < 12 || &webp_data[0..4] != b"RIFF" || &webp_data[8..12] != b"WEBP" {
            log::warn!(
                "[FOLDER PREVIEW CACHE] Invalid WebP header for {:?} ({} bytes)",
                folder_path.file_name(),
                webp_data.len()
            );
            return None;
        }

        // Decode WebP back to RGBA
        let decoder = webp::Decoder::new(&webp_data);
        let decoded = match decoder.decode() {
            Some(img) => img,
            None => {
                log::warn!(
                    "[FOLDER PREVIEW CACHE] WebP decode failed for {:?} ({} bytes)",
                    folder_path.file_name(),
                    webp_data.len()
                );
                return None;
            }
        };
        let rgba = decoded.to_image().to_rgba8();
        let (w, h) = (rgba.width(), rgba.height());
        Some((rgba.into_raw(), w, h, created_at))
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
        let expected_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4));
        if expected_len != Some(rgba_data.len()) {
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
}
