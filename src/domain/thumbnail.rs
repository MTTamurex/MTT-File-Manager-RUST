use crate::infrastructure::io_priority::IOPriority;
use std::path::PathBuf;

/// Thumbnail data extracted from file
#[derive(Clone)]
pub struct ThumbnailData {
    pub path: PathBuf,
    pub image_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub generation: usize,
    pub priority: IOPriority,
    pub not_found: bool,
}

/// Logical pixel size requested for the detail/preview panel.
pub const DETAIL_PREVIEW_SIZE: u32 = 512;

/// Logical pixel size for GIF previews in the detail panel (lower to save memory).
pub const DETAIL_PREVIEW_GIF_SIZE: u32 = 256;

/// Returns the logical pixel size the detail panel needs for `path`.
/// GIFs use a smaller target; everything else uses the full preview size.
pub fn detail_preview_size(path: &std::path::Path) -> u32 {
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("gif"))
    {
        DETAIL_PREVIEW_GIF_SIZE
    } else {
        DETAIL_PREVIEW_SIZE
    }
}
