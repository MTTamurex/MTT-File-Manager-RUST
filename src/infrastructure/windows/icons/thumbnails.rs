use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::{
    core::*, Win32::Foundation::*, Win32::Graphics::Gdi::*, Win32::System::Com::*,
    Win32::UI::Shell::*,
};

/// Extracts a Windows thumbnail for a file using IShellItemImageFactory.
///
/// # Safety
/// Uses COM interfaces (IShellItem, IShellItemImageFactory).
/// DeleteObject is called on the returned HBITMAP.
pub fn extract_thumbnail(
    path: &Path,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        let path_str = path.to_string_lossy().to_string();
        let path_wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();

        let shell_item: IShellItem = SHCreateItemFromParsingName(PCWSTR(path_wide.as_ptr()), None)?;

        let image_factory: IShellItemImageFactory = shell_item.cast()?;

        let size = SIZE { cx: 256, cy: 256 };
        let hbitmap: HBITMAP = image_factory.GetImage(size, SIIGBF_THUMBNAILONLY)?;

        let (rgba_data, width, height) =
            crate::infrastructure::windows::bitmap_conversion::hbitmap_to_rgba(hbitmap)?;

        let _ = DeleteObject(hbitmap.into());

        Ok((rgba_data, width, height))
    }
}

/// Extracts Windows folder preview (sandwich effect) using IShellItemImageFactory.
///
/// Returns the folder icon with internal content preview composed by Windows Shell.
/// Falls back to standard folder icon if no preview content available.
///
/// # Safety
/// Uses COM interfaces (IShellItem, IShellItemImageFactory).
/// DeleteObject is called on the returned HBITMAP.
pub fn get_folder_preview(
    folder_path: &Path,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        // SAFETY: path_wide is valid for the duration of this call
        let path_wide: Vec<u16> = folder_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let shell_item: IShellItem = SHCreateItemFromParsingName(PCWSTR(path_wide.as_ptr()), None)?;
        let image_factory: IShellItemImageFactory = shell_item.cast()?;

        // Request 256px with SIIGBF_THUMBNAILONLY to get the sandwich preview
        let size = SIZE { cx: 256, cy: 256 };

        match image_factory.GetImage(size, SIIGBF_THUMBNAILONLY) {
            Ok(hbitmap) => {
                // SAFETY: hbitmap is valid, DeleteObject called after conversion
                let result =
                    crate::infrastructure::windows::bitmap_conversion::hbitmap_to_rgba(hbitmap)?;
                let _ = DeleteObject(hbitmap.into());
                Ok(result)
            }
            Err(_) => {
                // Fallback: Get standard folder icon without preview content
                let hbitmap = image_factory.GetImage(size, SIIGBF_ICONONLY)?;
                let result =
                    crate::infrastructure::windows::bitmap_conversion::hbitmap_to_rgba(hbitmap)?;
                let _ = DeleteObject(hbitmap.into());
                Ok(result)
            }
        }
    }
}

/// Forces extraction of folder preview, bypassing Windows thumbnail cache.
///
/// Uses IThumbnailCache with WTS_FORCEEXTRACTION flag to ensure we get a fresh preview.
/// This is useful when the cached folder preview has black background or is corrupted.
///
/// # Safety
/// Uses COM interfaces (IShellItem, IThumbnailCache, ISharedBitmap).
/// All COM objects are properly released.
pub fn force_extract_folder_preview(
    folder_path: &Path,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    use windows::Win32::UI::Shell::{
        ISharedBitmap, IThumbnailCache, LocalThumbnailCache, WTS_CACHEFLAGS, WTS_FORCEEXTRACTION,
        WTS_SCALETOREQUESTEDSIZE,
    };

    unsafe {
        // SAFETY: path_wide is valid for the duration of this call
        let path_wide: Vec<u16> = folder_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // Create IShellItem for the folder
        let shell_item: IShellItem = SHCreateItemFromParsingName(PCWSTR(path_wide.as_ptr()), None)?;

        // Create IThumbnailCache instance
        let thumbnail_cache: IThumbnailCache =
            CoCreateInstance(&LocalThumbnailCache, None, CLSCTX_INPROC_SERVER)?;

        // Request thumbnail with FORCE EXTRACTION (ignores cache)
        let flags = WTS_FORCEEXTRACTION | WTS_SCALETOREQUESTEDSIZE;
        let requested_size: u32 = 256;

        let mut shared_bitmap: Option<ISharedBitmap> = None;
        let mut _cache_flags: WTS_CACHEFLAGS = WTS_CACHEFLAGS::default();
        let mut _thumbnail_id = windows::Win32::UI::Shell::WTS_THUMBNAILID::default();

        thumbnail_cache.GetThumbnail(
            &shell_item,
            requested_size,
            flags,
            Some(&mut shared_bitmap),
            Some(&mut _cache_flags),
            Some(&mut _thumbnail_id),
        )?;

        // Extract HBITMAP from shared bitmap
        let bitmap = shared_bitmap.ok_or("No bitmap returned")?;
        let hbitmap = bitmap.GetSharedBitmap()?;

        // Convert to RGBA
        let result =
            crate::infrastructure::windows::bitmap_conversion::hbitmap_to_rgba(HBITMAP(hbitmap.0))?;

        // Cleanup: ISharedBitmap is released when dropped (Rust RAII)
        // HBITMAP ownership remains with ISharedBitmap, don't delete it

        Ok(result)
    }
}

/// Forces extraction of a new thumbnail, bypassing the Windows thumbnail cache.
///
/// Uses IThumbnailCache::GetThumbnail with WTS_FORCEEXTRACTION flag.
/// This is useful when the cached thumbnail is corrupted or shows an icon instead of content.
///
/// IMPORTANT: Single attempt only - if it fails, Stage 5 (Media Foundation) will handle it.
///
/// # Safety
/// Uses COM interfaces (IShellItem, IThumbnailCache, ISharedBitmap).
/// All COM objects are properly released.
pub fn force_extract_thumbnail(
    path: &Path,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    force_extract_thumbnail_with_size(path, None)
}

pub fn force_extract_thumbnail_with_size(
    path: &Path,
    requested_size: Option<u32>,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    use windows::Win32::UI::Shell::{
        ISharedBitmap, IThumbnailCache, LocalThumbnailCache, WTS_CACHEFLAGS, WTS_FORCEEXTRACTION,
        WTS_SCALETOREQUESTEDSIZE,
    };

    unsafe {
        // SAFETY: path_wide is valid for the duration of this call
        let path_wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // Create IShellItem for the file
        let shell_item: IShellItem = SHCreateItemFromParsingName(PCWSTR(path_wide.as_ptr()), None)?;

        // Create IThumbnailCache instance
        let thumbnail_cache: IThumbnailCache =
            CoCreateInstance(&LocalThumbnailCache, None, CLSCTX_INPROC_SERVER)?;

        // Request thumbnail with FORCE EXTRACTION (ignores cache)
        // WTS_FORCEEXTRACTION = 0x8 - Forces extraction even if cached
        // WTS_SCALETOREQUESTEDSIZE = 0x100 - Scales to requested size
        let flags = WTS_FORCEEXTRACTION | WTS_SCALETOREQUESTEDSIZE;
        let requested_size = requested_size.unwrap_or(512).clamp(1, 1024);

        // Single attempt - no retries. Stage 5 (Media Foundation) handles failures.
        let mut shared_bitmap: Option<ISharedBitmap> = None;
        let mut _cache_flags: WTS_CACHEFLAGS = WTS_CACHEFLAGS::default();
        let mut _thumbnail_id = windows::Win32::UI::Shell::WTS_THUMBNAILID::default();

        thumbnail_cache.GetThumbnail(
            &shell_item,
            requested_size,
            flags,
            Some(&mut shared_bitmap),
            Some(&mut _cache_flags),
            Some(&mut _thumbnail_id),
        )?;

        if let Some(bitmap) = shared_bitmap {
            // Get HBITMAP from ISharedBitmap
            let hbitmap = bitmap.GetSharedBitmap()?;

            // Convert to RGBA
            let rgba_result =
                crate::infrastructure::windows::bitmap_conversion::hbitmap_to_rgba(hbitmap)?;
            return Ok(rgba_result);
        }

        Err("No bitmap returned from IThumbnailCache".into())
    }
}
