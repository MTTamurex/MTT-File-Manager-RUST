//! Windows icon extraction functions
//! Follows .cursorrules: single responsibility, < 300 lines

use std::path::{Path, PathBuf};
use windows::{
    core::*, Win32::Foundation::*, Win32::Graphics::Gdi::*, Win32::Storage::FileSystem::*,
    Win32::System::Com::*, Win32::UI::Shell::*, Win32::UI::WindowsAndMessaging::*,
};

use crate::domain::file_entry::IconSize;

/// Extracts the "This PC" (My Computer) icon using PIDL (robust method).
///
/// # Safety
/// Uses SHGetSpecialFolderLocation and SHGetFileInfoW with PIDL.
/// Always frees the PIDL with CoTaskMemFree.
pub fn extract_computer_icon(
    size: IconSize,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        // 1. Get PIDL for "My Computer" (CSIDL_DRIVES)
        let pidl = match SHGetSpecialFolderLocation(Some(HWND::default()), CSIDL_DRIVES as i32)
        {
            Ok(p) => p,
            Err(_) => {
                return Err("Failed to get PIDL for My Computer".into());
            }
        };

        let mut shfi = SHFILEINFOW::default();

        // 2. Flags with SHGFI_PIDL (CRITICAL!)
        let flags = SHGFI_PIDL
            | SHGFI_ICON
            | match size {
                IconSize::Small => SHGFI_SMALLICON,
                IconSize::Large | IconSize::Jumbo => SHGFI_LARGEICON,
            };

        // 3. Request icon using PIDL (cast to PCWSTR as required by API)
        let result = SHGetFileInfoW(
            PCWSTR(pidl as *const u16),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        );

        // 4. Free PIDL (ALWAYS! To avoid memory leak)
        CoTaskMemFree(Some(pidl as *const std::ffi::c_void));

        if result == 0 || shfi.hIcon.is_invalid() {
            return Err("Failed to get computer icon".into());
        }

        // 5. Convert and cleanup icon
        let hicon = shfi.hIcon;
        let conversion_result = super::bitmap_conversion::hicon_to_rgba(hicon);

        let _ = DestroyIcon(hicon);

        conversion_result
    }
}

/// Extracts a Windows thumbnail for a file using IShellItemImageFactory.
///
/// # Safety
/// Uses COM interfaces (IShellItem, IShellItemImageFactory).
/// DeleteObject is called on the returned HBITMAP.
pub fn extract_thumbnail(
    path: &PathBuf,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        let path_str = path.to_string_lossy().to_string();
        let path_wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();

        let shell_item: IShellItem = SHCreateItemFromParsingName(PCWSTR(path_wide.as_ptr()), None)?;

        let image_factory: IShellItemImageFactory = shell_item.cast()?;

        let size = SIZE { cx: 256, cy: 256 };
        let hbitmap: HBITMAP = image_factory.GetImage(size, SIIGBF_THUMBNAILONLY)?;

        let (rgba_data, width, height) = super::bitmap_conversion::hbitmap_to_rgba(hbitmap)?;

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
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let shell_item: IShellItem = SHCreateItemFromParsingName(PCWSTR(path_wide.as_ptr()), None)?;
        let image_factory: IShellItemImageFactory = shell_item.cast()?;

        // Request 256px with SIIGBF_THUMBNAILONLY to get the sandwich preview
        let size = SIZE { cx: 256, cy: 256 };

        match image_factory.GetImage(size, SIIGBF_THUMBNAILONLY) {
            Ok(hbitmap) => {
                // SAFETY: hbitmap is valid, DeleteObject called after conversion
                let result = super::bitmap_conversion::hbitmap_to_rgba(hbitmap)?;
                let _ = DeleteObject(hbitmap.into());
                Ok(result)
            }
            Err(_) => {
                // Fallback: Get standard folder icon without preview content
                let hbitmap = image_factory.GetImage(size, SIIGBF_ICONONLY)?;
                let result = super::bitmap_conversion::hbitmap_to_rgba(hbitmap)?;
                let _ = DeleteObject(hbitmap.into());
                Ok(result)
            }
        }
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
    use windows::Win32::UI::Shell::{
        ISharedBitmap, IThumbnailCache, LocalThumbnailCache, WTS_CACHEFLAGS, WTS_FORCEEXTRACTION,
        WTS_SCALETOREQUESTEDSIZE,
    };

    unsafe {
        // SAFETY: path_wide is valid for the duration of this call
        let path_wide: Vec<u16> = path
            .to_string_lossy()
            .encode_utf16()
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
        let requested_size: u32 = 512; // Good balance for preview panel

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
            let rgba_result = super::bitmap_conversion::hbitmap_to_rgba(hbitmap)?;
            return Ok(rgba_result);
        }

        Err("No bitmap returned from IThumbnailCache".into())
    }
}

/// Extracts the native Windows icon for a file extension.
///
/// Uses FILE_ATTRIBUTE_NORMAL + SHGFI_USEFILEATTRIBUTES to get the default icon for the type.
///
/// # Safety
/// Uses SHGetFileInfoW with dummy path. HICON is always freed with DestroyIcon.
pub fn extract_file_icon(
    extension: &str, // ".pdf", ".exe", etc.
    size: IconSize,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        // Create dummy path with extension (e.g., "dummy.pdf")
        let dummy_path = format!("dummy{}", extension);
        let path_wide: Vec<u16> = dummy_path
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        // For Jumbo icons, use IShellItemImageFactory even with dummy path (if possible)
        // Note: SHCreateItemFromParsingName with dummy path rarely works for Jumbo.
        // We stick to SHGetFileInfo for dummy icons, but ensure we use Large if Jumbo requested.
        
        let mut shfi = SHFILEINFOW::default();

        // CORRECT FLAGS: USEFILEATTRIBUTES allows dummy path
        let flags = SHGFI_ICON
            | SHGFI_USEFILEATTRIBUTES
            | match size {
                IconSize::Small => SHGFI_SMALLICON,
                IconSize::Large | IconSize::Jumbo => SHGFI_LARGEICON,
            };

        let result = SHGetFileInfoW(
            PCWSTR(path_wide.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        );

        if result == 0 || shfi.hIcon.is_invalid() {
            return Err("Failed to get file icon".into());
        }

        let hicon = shfi.hIcon;
        let conversion_result = super::bitmap_conversion::hicon_to_rgba(hicon);

        // SAFETY: Always free HICON
        let _ = DestroyIcon(hicon);

        conversion_result
    }
}

/// Extracts folder icon using DUMMY path.
///
/// Uses FILE_ATTRIBUTE_DIRECTORY + SHGFI_USEFILEATTRIBUTES to get the standard folder icon.
pub fn extract_folder_icon(
    size: IconSize,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        if matches!(size, IconSize::Jumbo) {
            // High quality folder icon using known folder ID if possible, 
            // or just a common path that is guaranteed to be a directory.
            let windows_dir = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
            let path_wide: Vec<u16> = windows_dir.encode_utf16().chain(std::iter::once(0)).collect();
            
            if let Ok(shell_item) = SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(path_wide.as_ptr()), None) {
                if let Ok(image_factory) = shell_item.cast::<IShellItemImageFactory>() {
                    let size_factory = SIZE { cx: 256, cy: 256 };
                    if let Ok(hbitmap) = image_factory.GetImage(size_factory, SIIGBF_ICONONLY) {
                        let res = super::bitmap_conversion::hbitmap_to_rgba(hbitmap)?;
                        let _ = DeleteObject(hbitmap.into());
                        return Ok(res);
                    }
                }
            }
        }

        let dummy_path = "dummy_folder";
        let path_wide: Vec<u16> = dummy_path
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let mut shfi = SHFILEINFOW::default();

        let flags = SHGFI_ICON
            | SHGFI_USEFILEATTRIBUTES
            | match size {
                IconSize::Small => SHGFI_SMALLICON,
                IconSize::Large | IconSize::Jumbo => SHGFI_LARGEICON,
            };

        let result = SHGetFileInfoW(
            PCWSTR(path_wide.as_ptr()),
            FILE_ATTRIBUTE_DIRECTORY,
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        );

        if result == 0 || shfi.hIcon.is_invalid() {
            return Err("Failed to get folder icon".into());
        }

        let hicon = shfi.hIcon;
        let conversion_result = super::bitmap_conversion::hicon_to_rgba(hicon);

        let _ = DestroyIcon(hicon);

        conversion_result
    }
}

/// Extracts icon from a REAL file using full path.
/// Used for executables (.exe, .lnk, .ico) that have unique icons.
pub fn extract_file_icon_by_path(
    path: &Path,
    size: IconSize,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        let path_wide: Vec<u16> = path
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        // For Jumbo icons, use IShellItemImageFactory (higher quality)
        if matches!(size, IconSize::Jumbo) {
            if let Ok(shell_item) = SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(path_wide.as_ptr()), None) {
                if let Ok(image_factory) = shell_item.cast::<IShellItemImageFactory>() {
                    let size_factory = SIZE { cx: 256, cy: 256 };
                    // SIIGBF_ICONONLY to ensure we get the icon and not a thumbnail if it were a file
                    if let Ok(hbitmap) = image_factory.GetImage(size_factory, SIIGBF_ICONONLY) {
                        let res = super::bitmap_conversion::hbitmap_to_rgba(hbitmap)?;
                        let _ = DeleteObject(hbitmap.into());
                        return Ok(res);
                    }
                }
            }
        }

        let mut shfi = SHFILEINFOW::default();

        // WITHOUT USEFILEATTRIBUTES - uses real file
        let flags = SHGFI_ICON
            | match size {
                IconSize::Small => SHGFI_SMALLICON,
                IconSize::Large | IconSize::Jumbo => SHGFI_LARGEICON,
            };

        let result = SHGetFileInfoW(
            PCWSTR(path_wide.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        );

        if result == 0 || shfi.hIcon.is_invalid() {
            return Err("Failed to get file icon".into());
        }

        let hicon = shfi.hIcon;
        let conversion_result = super::bitmap_conversion::hicon_to_rgba(hicon);

        let _ = DestroyIcon(hicon);

        conversion_result
    }
}

/// Extracts REAL icon from a drive (C:\, D:\, etc.).
///
/// Uses real path (not dummy) and WITHOUT SHGFI_USEFILEATTRIBUTES.
/// This forces Windows to return the specific drive icon (HD, SSD, USB, etc.).
pub fn extract_drive_icon(
    drive_path: &str,
    size: IconSize,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        let path_wide: Vec<u16> = drive_path
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        // For Jumbo icons, use IShellItemImageFactory (higher quality)
        if matches!(size, IconSize::Jumbo) {
            let shell_item: IShellItem =
                SHCreateItemFromParsingName(PCWSTR(path_wide.as_ptr()), None)?;

            let image_factory: IShellItemImageFactory = shell_item.cast()?;

            let size_factory = SIZE { cx: 256, cy: 256 };

            // SIIGBF_ICONONLY to ensure we get the icon and not a thumbnail if it were a file
            let hbitmap: HBITMAP = image_factory.GetImage(size_factory, SIIGBF_ICONONLY)?;

            let (rgba_data, width, height) = super::bitmap_conversion::hbitmap_to_rgba(hbitmap)?;
            let _ = DeleteObject(hbitmap.into());
            return Ok((rgba_data, width, height));
        }

        // For Small/Large, use legacy but fast SHGetFileInfo
        let mut shfi = SHFILEINFOW::default();

        let flags = SHGFI_ICON
            | match size {
                IconSize::Small => SHGFI_SMALLICON,
                IconSize::Large | IconSize::Jumbo => SHGFI_LARGEICON, // Fallback (Jumbo handled above)
            };

        let result = SHGetFileInfoW(
            PCWSTR(path_wide.as_ptr()),
            FILE_ATTRIBUTE_DIRECTORY,
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        );

        if result == 0 || shfi.hIcon.is_invalid() {
            return Err("Failed to get drive icon".into());
        }

        let hicon = shfi.hIcon;
        let conversion_result = super::bitmap_conversion::hicon_to_rgba(hicon);
        let _ = DestroyIcon(hicon);

        conversion_result
    }
}

/// Returns the default Windows icon for a file type (by extension), without requiring the file to exist.
/// Initializes COM for proper Shell integration on secondary threads.
///
/// REFACTORED: Now uses simple filename logic (file.ext) instead of fake absolute C:\ path,
/// which fixes issues with Recycle Bin items showing generic icons.
pub fn get_file_type_icon(
    is_folder: bool,
    extension: &str,
    size: IconSize,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Com::CoInitialize;
    use windows::Win32::UI::Shell::{
        SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SMALLICON,
        SHGFI_USEFILEATTRIBUTES,
    };

    // Debug: Verifique se a extensão está chegando limpa no console
    // println!("Buscando ícone para extensão: '{}', is_folder: {}", extension, is_folder);

    unsafe {
        // Inicializa COM para garantir acesso ao Registro do Windows
        let _ = CoInitialize(None);

        if matches!(size, IconSize::Jumbo) && is_folder {
             if let Ok(res) = extract_folder_icon(IconSize::Jumbo) {
                 return Ok(res);
             }
        }

        let mut shfi = SHFILEINFOW::default();

        let flags = SHGFI_ICON
            | SHGFI_USEFILEATTRIBUTES
            | match size {
                IconSize::Small => SHGFI_SMALLICON,
                IconSize::Large | IconSize::Jumbo => SHGFI_LARGEICON,
            };

        let file_attributes = if is_folder {
            windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY
        } else {
            windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL
        };

        // ESTRATÉGIA CORRIGIDA:
        // Não use caminhos absolutos (C:\...). Use apenas um nome simples.
        let dummy_name = if is_folder {
            "folder".to_string()
        } else {
            // Remove pontos extras e garante um único ponto
            let clean_ext = extension.trim_start_matches('.');
            format!("file.{}", clean_ext) // ex: "file.rar"
        };

        let wide_path: Vec<u16> = dummy_name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let result = SHGetFileInfoW(
            PCWSTR(wide_path.as_ptr()),
            file_attributes,
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        );

        if result == 0 || shfi.hIcon.is_invalid() {
            return Err(format!("Falha SHGetFileInfoW para: {}", dummy_name).into());
        }

        let hicon = shfi.hIcon;

        // Reutilize a função hicon_to_rgba que já implementamos e funciona
        let conversion_result = super::bitmap_conversion::hicon_to_rgba(hicon);

        let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(hicon);

        conversion_result
    }
}

/// Extracts the Recycle Bin icon using SHGetKnownFolderIDList
pub fn extract_recycle_bin_icon(
    size: IconSize,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    use windows::Win32::UI::Shell::{
        FOLDERID_RecycleBinFolder, SHGetFileInfoW, SHGetKnownFolderIDList, SHFILEINFOW, SHGFI_ICON,
        SHGFI_LARGEICON, SHGFI_PIDL, SHGFI_SMALLICON,
    };

    unsafe {
        // Get the PIDL for the Recycle Bin
        let pidl = SHGetKnownFolderIDList(&FOLDERID_RecycleBinFolder, 0, None)?;

        let mut shfi = SHFILEINFOW::default();

        let flags = SHGFI_PIDL
            | SHGFI_ICON
            | match size {
                IconSize::Small => SHGFI_SMALLICON,
                IconSize::Large | IconSize::Jumbo => SHGFI_LARGEICON,
            };

        let result = SHGetFileInfoW(
            PCWSTR(pidl as *const u16),
            windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        );

        // Free the PIDL
        windows::Win32::System::Com::CoTaskMemFree(Some(pidl as *mut _));

        if result == 0 || shfi.hIcon.is_invalid() {
            return Err("Failed to get recycle bin icon".into());
        }

        let hicon = shfi.hIcon;
        let conversion_result = super::bitmap_conversion::hicon_to_rgba(hicon);
        let _ = DestroyIcon(hicon);

        conversion_result
    }
}
