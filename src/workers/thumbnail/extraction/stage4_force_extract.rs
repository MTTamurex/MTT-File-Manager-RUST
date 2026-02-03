//! Stage 4: Force thumbnail extraction
//!
//! Uses IThumbnailCache with WTS_FORCEEXTRACTION flag to bypass Windows thumbnail cache.
//! This is useful when the Windows cache returns an icon instead of a real thumbnail.

use std::path::Path;

/// Force extract thumbnail bypassing Windows cache
///
/// Uses WTS_FORCEEXTRACTION flag to ensure we get a real thumbnail, not a cached icon.
/// This is a "single attempt" stage - if it fails, Stage 5 takes over.
pub fn extract(path: &Path) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    crate::infrastructure::windows::icons::force_extract_thumbnail(path)
}