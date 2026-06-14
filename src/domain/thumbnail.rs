use crate::infrastructure::io_priority::IOPriority;
use std::path::PathBuf;
use std::sync::Arc;

/// Convert un-premultiplied RGBA data to premultiplied RGBA in place.
///
/// Each pixel's RGB channels are multiplied by the alpha channel (normalized to [0,1]),
/// which is the format expected by `egui::ColorImage::from_rgba_premultiplied`.
/// This avoids the per-pixel alpha premultiplication cost on the UI thread.
///
/// Originally the UI thread called `ColorImage::from_rgba_unmultiplied` which does this
/// conversion inline for every thumbnail upload. Moving it to the worker thread reduces
/// UI stall time, especially on OpenGL backends where each `ctx.load_texture` call blocks
/// the CPU until the driver finishes the transfer.
pub fn premultiply_rgba_in_place(rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        let a = pixel[3] as u32;
        if a == 0 {
            pixel[0] = 0;
            pixel[1] = 0;
            pixel[2] = 0;
        } else if a != 255 {
            pixel[0] = ((pixel[0] as u32 * a + 128) / 255) as u8;
            pixel[1] = ((pixel[1] as u32 * a + 128) / 255) as u8;
            pixel[2] = ((pixel[2] as u32 * a + 128) / 255) as u8;
        }
        // a == 255: no change needed — already fully opaque
    }
}

/// Thumbnail data extracted from file
#[derive(Clone)]
pub struct ThumbnailData {
    pub path: PathBuf,
    pub image_data: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub generation: usize,
    pub request_epoch: u64,
    pub priority: IOPriority,
    pub not_found: bool,
    /// When `true`, `image_data` contains premultiplied-alpha RGBA pixels
    /// and should be uploaded with `ColorImage::from_rgba_premultiplied`
    /// instead of `ColorImage::from_rgba_unmultiplied`.
    pub premultiplied: bool,
}

/// Logical pixel size requested for the detail/preview panel.
pub const MAX_THUMBNAIL_SIDE: u32 = 512;

/// Logical pixel size requested for the detail/preview panel.
pub const DETAIL_PREVIEW_SIZE: u32 = MAX_THUMBNAIL_SIDE;

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
