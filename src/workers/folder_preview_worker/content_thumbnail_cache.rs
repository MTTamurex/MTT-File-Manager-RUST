use crate::infrastructure::disk_cache::{ThumbnailCacheEntry, ThumbnailDiskCache};
use crate::workers::thumbnail::processing::resize_to_bucket;
use image::ImageFormat;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn try_cached_content_thumbnail(
    disk_cache: &ThumbnailDiskCache,
    media_path: &Path,
    media_modified: Option<SystemTime>,
    bucket_size: u32,
) -> Option<(Vec<u8>, u32, u32)> {
    let requested_modified = media_modified
        .and_then(|mtime| mtime.duration_since(UNIX_EPOCH).ok())
        .map(|dur| dur.as_secs())
        .unwrap_or(0);

    if let Some(modified) = media_modified {
        if let Some(entry) = disk_cache.get(media_path, modified) {
            if let Some(decoded) = decode_content_thumbnail_cache_entry(entry, bucket_size) {
                return Some(decoded);
            }
        }
    }

    let entry = disk_cache.get_latest(media_path)?;
    if requested_modified > 0 && entry.modified_at > 0 && entry.modified_at != requested_modified {
        return None;
    }

    decode_content_thumbnail_cache_entry(entry, bucket_size)
}

fn decode_content_thumbnail_cache_entry(
    entry: ThumbnailCacheEntry,
    bucket_size: u32,
) -> Option<(Vec<u8>, u32, u32)> {
    if !entry.satisfies_request(bucket_size) {
        return None;
    }

    let img = image::load_from_memory_with_format(&entry.data, ImageFormat::WebP).ok()?;
    let rgba = img.to_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    Some(resize_to_bucket(
        rgba.into_raw(),
        width,
        height,
        bucket_size,
    ))
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
    fn cached_content_thumbnail_satisfies_smaller_folder_preview_bucket() {
        let dir = tempdir().expect("create temp dir");
        let cache = ThumbnailDiskCache::new(dir.path().to_path_buf()).expect("create cache");
        let media_path = dir.path().join("cover.jpg");
        let modified = UNIX_EPOCH + Duration::from_secs(42);

        cache
            .put(
                &media_path,
                modified,
                512,
                &rgba(512, 512, [255, 0, 0, 255]),
                512,
                512,
            )
            .expect("cache thumbnail");

        let (data, width, height) =
            try_cached_content_thumbnail(&cache, &media_path, Some(modified), 128)
                .expect("larger cached thumbnail should satisfy folder preview content");

        assert_eq!(width, 128);
        assert_eq!(height, 128);
        assert_eq!(data.len(), 128 * 128 * 4);
    }
}
