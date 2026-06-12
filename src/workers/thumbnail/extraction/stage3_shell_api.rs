//! Stage 3: Windows Shell API extraction
//!
//! Uses the Windows Shell IShellItemImageFactory for universal thumbnail extraction.
//! This works for most file types including videos, documents, and executables.

use crate::domain::thumbnail::MAX_THUMBNAIL_SIDE;
use crate::infrastructure::windows::file_type::is_video_extension;
use std::path::Path;
use windows::core::Interface;
use windows::{
    core::PCWSTR,
    Win32::Graphics::Gdi::{DeleteObject, HBITMAP},
    Win32::UI::Shell::{
        IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_RESIZETOFIT,
        SIIGBF_THUMBNAILONLY,
    },
};

/// Extract thumbnail using Windows Shell API
///
/// This is the universal fallback that works for most file types.
/// For videos, uses THUMBNAILONLY to fail if only an icon is available.
pub fn extract(path: &Path) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    extract_with_size(path, None)
}

pub fn extract_with_size(
    path: &Path,
    requested_size: Option<u32>,
) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    // Thumbnail extraction is capped at 512px for every media type.
    let is_video = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| is_video_extension(&ext.to_lowercase()))
        .unwrap_or(false);

    let default_size_px = MAX_THUMBNAIL_SIDE;
    let size_px = requested_size_px(requested_size, default_size_px) as i32;

    unsafe {
        // SAFETY: Raw pointers from `path_wide` are valid for the call.
        // HBITMAP is a resource that is manually deleted with `DeleteObject` below.
        let path_str = path.to_string_lossy().to_string();
        let path_wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();

        let shell_item: IShellItem = SHCreateItemFromParsingName(PCWSTR(path_wide.as_ptr()), None)?;
        let image_factory: IShellItemImageFactory = shell_item.cast()?;

        let size = windows::Win32::Foundation::SIZE {
            cx: size_px,
            cy: size_px,
        };

        // For videos: use THUMBNAILONLY to FAIL if only an icon is available
        // This allows Stage 4 (force extraction) to be triggered
        // For other files: use RESIZETOFIT which accepts icons
        let flags = if is_video {
            SIIGBF_THUMBNAILONLY
        } else {
            SIIGBF_RESIZETOFIT
        };
        let hbitmap: HBITMAP = image_factory.GetImage(size, flags)?;

        let result = crate::infrastructure::windows::bitmap_conversion::hbitmap_to_rgba(hbitmap);
        let _ = DeleteObject(hbitmap.into());
        Ok(result?)
    }
}

fn requested_size_px(requested_size: Option<u32>, default_size_px: u32) -> u32 {
    requested_size
        .unwrap_or(default_size_px)
        .clamp(1, default_size_px.min(MAX_THUMBNAIL_SIDE).max(1))
}

#[cfg(test)]
mod tests {
    use super::requested_size_px;

    #[test]
    fn requested_size_px_uses_target_when_available() {
        assert_eq!(requested_size_px(Some(512), 512), 512);
        assert_eq!(requested_size_px(Some(2048), 512), 512);
        assert_eq!(requested_size_px(None, 512), 512);
    }
}
