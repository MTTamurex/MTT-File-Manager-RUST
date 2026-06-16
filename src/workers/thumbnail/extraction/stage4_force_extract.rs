//! Stage 4: Force thumbnail extraction
//!
//! Uses IThumbnailCache with WTS_EXTRACTDONOTCACHE so fallback extraction does
//! not populate the Windows Explorer thumbnail cache.

use std::path::Path;

/// Force extract thumbnail bypassing Windows cache
///
/// Uses WTS_EXTRACTDONOTCACHE to ensure the result is persisted only by the app.
/// This is a "single attempt" stage - if it fails, Stage 5 takes over.
pub fn extract(path: &Path) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    crate::infrastructure::windows::icons::extract_thumbnail_without_windows_cache(path)
}

pub fn extract_with_size(
    path: &Path,
    requested_size: Option<u32>,
) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    crate::infrastructure::windows::icons::extract_thumbnail_without_windows_cache_with_size(
        path,
        requested_size,
    )
}
