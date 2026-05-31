//! Stage 1: Image crate extraction (Fast Path)
//!
//! Uses the `image` crate for fast decoding of common image formats.
//! This is the fastest path for standard image files.

use crate::infrastructure::io_priority::IOPriority;
use crate::infrastructure::windows::file_flags::{
    open_sequential, open_sequential_background, open_sequential_low_priority,
};
use image::ImageFormat;
use std::io::BufReader;
use std::path::Path;

/// Try to extract thumbnail using the image crate
///
/// Supports: jpg, jpeg, png, bmp, gif, webp, tiff
pub fn extract(path: &Path, priority: IOPriority) -> Option<(Vec<u8>, u32, u32)> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if !matches!(
        ext.as_str(),
        "jpg" | "jpeg" | "png" | "bmp" | "gif" | "webp" | "tiff"
    ) {
        return None;
    }

    let file = match priority {
        IOPriority::Interactive => open_sequential(path).ok()?,
        IOPriority::Prefetch => open_sequential_low_priority(path).ok()?,
        IOPriority::Background => open_sequential_background(path).ok()?,
    };
    let reader = BufReader::with_capacity(65536, file);
    let format = ImageFormat::from_extension(&ext)?;

    match image::load(reader, format) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let width = rgba.width();
            let height = rgba.height();
            Some((rgba.into_vec(), width, height))
        }
        Err(_) => None,
    }
}
