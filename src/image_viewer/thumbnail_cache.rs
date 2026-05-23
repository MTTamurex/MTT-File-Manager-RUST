use crate::image_viewer::loader::DecodedFrame;
use crate::infrastructure::disk_cache::{ThumbnailCacheEntry, ThumbnailDiskCache};
use image::imageops::FilterType;
use once_cell::sync::Lazy;
use std::path::{Path, PathBuf};

static VIEWER_THUMBNAIL_CACHE: Lazy<Option<ThumbnailDiskCache>> = Lazy::new(|| {
    let cache_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("MTT-File-Manager")
        .join("thumbnails");

    match ThumbnailDiskCache::new(cache_dir) {
        Ok(cache) => Some(cache),
        Err(err) => {
            log::warn!(
                "[IMAGE-VIEWER] failed to open thumbnail cache for fast preview path: {}",
                err
            );
            None
        }
    }
});

pub(super) fn try_fast_preview_from_disk_cache(path: &Path, max_side: u32) -> Option<DecodedFrame> {
    let cache = VIEWER_THUMBNAIL_CACHE.as_ref()?;
    decode_cache_entry(cache.get_latest(path)?, max_side)
}

pub(super) fn try_fast_previews_from_disk_cache(
    paths: &[PathBuf],
    max_side: u32,
) -> Vec<Option<DecodedFrame>> {
    if paths.is_empty() {
        return Vec::new();
    }

    let Some(cache) = VIEWER_THUMBNAIL_CACHE.as_ref() else {
        return vec![None; paths.len()];
    };

    cache
        .get_latest_batch(paths)
        .into_iter()
        .map(|entry| entry.and_then(|entry| decode_cache_entry(entry, max_side)))
        .collect()
}

fn decode_cache_entry(entry: ThumbnailCacheEntry, max_side: u32) -> Option<DecodedFrame> {
    let image = image::load_from_memory_with_format(&entry.data, image::ImageFormat::WebP).ok()?;
    let image = if max_side > 0 && (image.width() > max_side || image.height() > max_side) {
        image.resize(max_side, max_side, FilterType::Triangle)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    let width = rgba.width();
    let height = rgba.height();

    Some(DecodedFrame {
        rgba: rgba.into_raw(),
        width,
        height,
        original_width: width,
        original_height: height,
    })
}
