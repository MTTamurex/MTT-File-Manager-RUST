use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::{core::*, Win32::Graphics::Gdi::*, Win32::System::Com::*, Win32::UI::Shell::*};

/// Extracts a Windows thumbnail without adding it to the Windows thumbnail cache.
///
/// # Safety
/// Uses COM interfaces (IShellItem, IThumbnailCache, ISharedBitmap).
pub fn extract_thumbnail(
    path: &Path,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    extract_thumbnail_without_windows_cache(path)
}

/// Extracts Windows folder preview without adding it to the Windows thumbnail cache.
///
/// Returns the folder icon with internal content preview composed by Windows Shell.
///
/// # Safety
/// Uses COM interfaces (IShellItem, IThumbnailCache, ISharedBitmap).
pub fn get_folder_preview(
    folder_path: &Path,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    force_extract_folder_preview(folder_path)
}

/// Extracts folder preview without adding it to the Windows thumbnail cache.
///
/// Uses IThumbnailCache with WTS_EXTRACTDONOTCACHE so app-generated thumbnails
/// are persisted only in the app-owned cache.
///
/// # Safety
/// Uses COM interfaces (IShellItem, IThumbnailCache, ISharedBitmap).
/// All COM objects are properly released.
pub fn force_extract_folder_preview(
    folder_path: &Path,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    use windows::Win32::UI::Shell::{
        ISharedBitmap, IThumbnailCache, LocalThumbnailCache, WTS_CACHEFLAGS, WTS_EXTRACTDONOTCACHE,
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

        // Extract without adding the result to Explorer's thumbnail cache.
        let flags = WTS_EXTRACTDONOTCACHE;
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

/// Extracts a thumbnail without adding it to the Windows thumbnail cache.
///
/// Uses IThumbnailCache::GetThumbnail with WTS_EXTRACTDONOTCACHE so the app can
/// persist the result only in its own disk cache.
///
/// IMPORTANT: Single attempt only - if it fails, Stage 5 (Media Foundation) will handle it.
///
/// # Safety
/// Uses COM interfaces (IShellItem, IThumbnailCache, ISharedBitmap).
/// All COM objects are properly released.
pub fn extract_thumbnail_without_windows_cache(
    path: &Path,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    extract_thumbnail_without_windows_cache_with_size(path, None)
}

pub fn extract_thumbnail_without_windows_cache_with_size(
    path: &Path,
    requested_size: Option<u32>,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    use windows::Win32::UI::Shell::{
        ISharedBitmap, IThumbnailCache, LocalThumbnailCache, WTS_CACHEFLAGS, WTS_EXTRACTDONOTCACHE,
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

        // Extract without adding the result to Explorer's thumbnail cache.
        // WTS_EXTRACTDONOTCACHE must be used by itself per the Win32 API docs.
        let flags = WTS_EXTRACTDONOTCACHE;
        let requested_size = requested_size
            .unwrap_or(crate::domain::thumbnail::MAX_THUMBNAIL_SIDE)
            .clamp(1, crate::domain::thumbnail::MAX_THUMBNAIL_SIDE);

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

/// Legacy name retained for existing call sites. This does not populate the
/// Windows thumbnail cache; extracted pixels are cached only by the app.
pub fn force_extract_thumbnail(
    path: &Path,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    extract_thumbnail_without_windows_cache(path)
}

/// Legacy name retained for existing call sites. This does not populate the
/// Windows thumbnail cache; extracted pixels are cached only by the app.
pub fn force_extract_thumbnail_with_size(
    path: &Path,
    requested_size: Option<u32>,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    extract_thumbnail_without_windows_cache_with_size(path, requested_size)
}
