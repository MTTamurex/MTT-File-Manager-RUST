use super::{ThumbnailDiskCache, FOLDER_PREVIEW_CACHE_FORMAT_VERSION};
use image::{imageops, DynamicImage, ImageBuffer, Rgba};
use rusqlite::params;
use std::path::Path;

mod freshness;
mod variants;
mod writer;
pub use freshness::FolderPreviewSource;
pub(super) use variants::initialize_variant_table;
pub use writer::FolderPreviewPixels;

fn resize_rgba_to_width(
    rgba_data: Vec<u8>,
    width: u32,
    height: u32,
    target_width: u32,
) -> Option<(Vec<u8>, u32, u32)> {
    if width == 0 || height == 0 || target_width == 0 {
        return None;
    }

    if width <= target_width {
        return Some((rgba_data, width, height));
    }

    let expected_len = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    if rgba_data.len() != expected_len {
        return None;
    }

    let img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_raw(width, height, rgba_data)?;
    let resized = DynamicImage::ImageRgba8(img)
        .resize(target_width, u32::MAX, imageops::FilterType::CatmullRom)
        .to_rgba8();
    let (w, h) = (resized.width(), resized.height());
    Some((resized.into_raw(), w, h))
}

impl ThumbnailDiskCache {
    /// Retrieves a cached folder preview (Shell sandwich icon) from SQLite.
    /// Returns decoded straight-alpha RGBA data, plus the cache timestamp.
    /// The `created_at` (Unix seconds) allows callers to detect stale entries
    /// by comparing against the folder's last-write time.
    /// [READER]
    pub fn get_folder_preview_cache(
        &self,
        folder_path: &Path,
        bucket_size: u32,
    ) -> Option<(Vec<u8>, u32, u32, i64)> {
        self.get_folder_preview_for_appearance(folder_path, bucket_size, Default::default())
    }

    pub fn get_folder_preview_for_appearance(
        &self,
        folder_path: &Path,
        bucket_size: u32,
        appearance: crate::domain::folder_appearance::FolderAppearance,
    ) -> Option<(Vec<u8>, u32, u32, i64)> {
        self.get_folder_preview_inner(folder_path, bucket_size, appearance, None)
    }

    pub fn get_folder_preview_for_source(
        &self,
        folder_path: &Path,
        bucket_size: u32,
        appearance: crate::domain::folder_appearance::FolderAppearance,
        source: &FolderPreviewSource,
    ) -> Option<(Vec<u8>, u32, u32, i64)> {
        if !source.is_current() {
            return None;
        }
        self.get_folder_preview_inner(
            folder_path,
            bucket_size,
            appearance,
            Some(source.fingerprint()),
        )
    }

    fn get_folder_preview_inner(
        &self,
        folder_path: &Path,
        bucket_size: u32,
        appearance: crate::domain::folder_appearance::FolderAppearance,
        source: Option<&str>,
    ) -> Option<(Vec<u8>, u32, u32, i64)> {
        let db = self.reader.lock();
        let mut stmt = db
            .prepare_cached(
                "SELECT data, width, height, bucket_size, created_at, format_version, source_fingerprint
                 FROM folder_preview_variants WHERE folder_path = ?1 AND format_version = ?2
                 UNION ALL
                 SELECT data, width, height, bucket_size, created_at, format_version, NULL
                 FROM folder_previews WHERE folder_path = ?1 AND format_version = ?2
                 LIMIT 1",
            )
            .ok()?;

        let folder_path_str = folder_path.to_string_lossy();
        let (
            webp_data,
            db_width,
            _db_height,
            cached_bucket,
            created_at,
            format_version,
            fingerprint,
        ): (Vec<u8>, u32, u32, u32, i64, i64, Option<String>) = match stmt.query_row(
            params![
                &*folder_path_str,
                appearance.cache_version(FOLDER_PREVIEW_CACHE_FORMAT_VERSION)
            ],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)? as u32,
                    row.get::<_, i64>(2)? as u32,
                    row.get::<_, i64>(3)? as u32,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            },
        ) {
            Ok(row) => row,
            Err(_) => return None,
        };

        if format_version != appearance.cache_version(FOLDER_PREVIEW_CACHE_FORMAT_VERSION) {
            return None;
        }
        if source.is_some() && source != fingerprint.as_deref() {
            return None;
        }

        if cached_bucket < bucket_size && db_width < bucket_size {
            return None;
        }

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
        if w < bucket_size {
            return None;
        }

        let (rgba_data, w, h) = resize_rgba_to_width(rgba.into_raw(), w, h, bucket_size)?;
        Some((rgba_data, w, h, created_at))
    }

    /// Removes a cached folder preview.
    /// [WRITER]
    pub fn remove_folder_preview_cache(&self, folder_path: &Path) {
        let db = self.writer.lock();
        let _ = db.execute(
            "DELETE FROM folder_previews WHERE folder_path = ?",
            [folder_path.to_string_lossy()],
        );
        let _ = db.execute(
            "DELETE FROM folder_preview_variants WHERE folder_path = ?",
            [folder_path.to_string_lossy()],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn rgba(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            out.extend_from_slice(&color);
        }
        out
    }

    #[test]
    fn get_folder_preview_cache_reuses_larger_bucket() {
        let dir = tempdir().expect("create temp dir");
        let cache = ThumbnailDiskCache::new(dir.path().to_path_buf()).expect("create cache");
        let folder = dir.path().join("folder");
        std::fs::create_dir(&folder).expect("create folder");

        cache.put_folder_preview_cache(&folder, 512, &rgba(512, 512, [255, 0, 0, 255]), 512, 512);

        let (data, width, height, _) = cache
            .get_folder_preview_cache(&folder, 128)
            .expect("larger cached preview should satisfy smaller request");

        assert_eq!(width, 128);
        assert_eq!(height, 128);
        assert_eq!(data.len(), 128 * 128 * 4);
    }

    #[test]
    fn put_folder_preview_cache_does_not_replace_fresh_larger_bucket() {
        let dir = tempdir().expect("create temp dir");
        let cache = ThumbnailDiskCache::new(dir.path().to_path_buf()).expect("create cache");
        let folder = dir.path().join("folder");
        std::fs::create_dir(&folder).expect("create folder");

        cache.put_folder_preview_cache(&folder, 512, &rgba(512, 512, [255, 0, 0, 255]), 512, 512);
        cache.put_folder_preview_cache(&folder, 128, &rgba(128, 128, [0, 255, 0, 255]), 128, 128);

        let (_, width, height, _) = cache
            .get_folder_preview_cache(&folder, 512)
            .expect("fresh larger cached preview should be preserved");

        assert_eq!(width, 512);
        assert_eq!(height, 512);
    }

    #[test]
    fn get_folder_preview_cache_ignores_legacy_format_entries() {
        let dir = tempdir().expect("create temp dir");
        let cache = ThumbnailDiskCache::new(dir.path().to_path_buf()).expect("create cache");
        let folder = dir.path().join("folder");
        std::fs::create_dir(&folder).expect("create folder");

        {
            let db = cache.writer.lock();
            db.execute(
                "INSERT INTO folder_previews (folder_path, data, width, height, bucket_size, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    folder.to_string_lossy().to_string(),
                    b"legacy-lossy-webp".to_vec(),
                    512i64,
                    512i64,
                    512i64,
                    1i64
                ],
            )
            .expect("insert legacy entry");
        }

        assert!(cache.get_folder_preview_cache(&folder, 512).is_none());

        cache.put_folder_preview_cache(&folder, 128, &rgba(128, 128, [0, 255, 0, 128]), 128, 128);

        let (_, width, height, _) = cache
            .get_folder_preview_cache(&folder, 128)
            .expect("new-format entry should replace legacy cache");

        assert_eq!(width, 128);
        assert_eq!(height, 128);
    }

    #[test]
    fn put_folder_preview_cache_preserves_partial_alpha_pixels() {
        let dir = tempdir().expect("create temp dir");
        let cache = ThumbnailDiskCache::new(dir.path().to_path_buf()).expect("create cache");
        let folder = dir.path().join("folder");
        std::fs::create_dir(&folder).expect("create folder");

        let mut original = Vec::with_capacity(16 * 16 * 4);
        for y in 0..16u8 {
            for x in 0..16u8 {
                original.extend_from_slice(&[
                    x.wrapping_mul(13),
                    y.wrapping_mul(17),
                    x.wrapping_add(y).wrapping_mul(7),
                    ((x as u16 * 11 + y as u16 * 19) % 254 + 1) as u8,
                ]);
            }
        }

        cache.put_folder_preview_cache(&folder, 16, &original, 16, 16);

        let (decoded, width, height, _) = cache
            .get_folder_preview_cache(&folder, 16)
            .expect("lossless folder preview cache should decode");

        assert_eq!(width, 16);
        assert_eq!(height, 16);
        assert_eq!(decoded, original);
    }
}
