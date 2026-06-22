use crate::domain::file_entry::IconSize;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::{
    core::*, Win32::Foundation::*, Win32::Graphics::Gdi::*, Win32::Storage::FileSystem::*,
    Win32::UI::Shell::*, Win32::UI::WindowsAndMessaging::*,
};

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
        let conversion_result =
            crate::infrastructure::windows::bitmap_conversion::hicon_to_rgba(hicon);

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
            let windows_dir =
                std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
            let path_wide: Vec<u16> = windows_dir
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();

            if let Ok(shell_item) =
                SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(path_wide.as_ptr()), None)
            {
                if let Ok(image_factory) = shell_item.cast::<IShellItemImageFactory>() {
                    let size_factory = SIZE { cx: 256, cy: 256 };
                    if let Ok(hbitmap) = image_factory.GetImage(size_factory, SIIGBF_ICONONLY) {
                        let res =
                            crate::infrastructure::windows::bitmap_conversion::hbitmap_to_rgba(
                                hbitmap,
                            );
                        let _ = DeleteObject(hbitmap.into());
                        return res;
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
        let conversion_result =
            crate::infrastructure::windows::bitmap_conversion::hicon_to_rgba(hicon);

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
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // For Jumbo icons, use IShellItemImageFactory (higher quality)
        if matches!(size, IconSize::Jumbo) {
            if let Ok(shell_item) =
                SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(path_wide.as_ptr()), None)
            {
                if let Ok(image_factory) = shell_item.cast::<IShellItemImageFactory>() {
                    let size_factory = SIZE { cx: 256, cy: 256 };
                    // SIIGBF_ICONONLY to ensure we get the icon and not a thumbnail if it were a file
                    if let Ok(hbitmap) = image_factory.GetImage(size_factory, SIIGBF_ICONONLY) {
                        let res =
                            crate::infrastructure::windows::bitmap_conversion::hbitmap_to_rgba(
                                hbitmap,
                            );
                        let _ = DeleteObject(hbitmap.into());
                        return res;
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
        let conversion_result =
            crate::infrastructure::windows::bitmap_conversion::hicon_to_rgba(hicon);

        let _ = DestroyIcon(hicon);

        conversion_result
    }
}

pub fn extract_icon_resource(
    resource: &str,
    size: IconSize,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    let Some((path, index)) = parse_icon_resource(resource) else {
        return Err("invalid icon resource".into());
    };

    unsafe {
        let path_wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut h_large = HICON::default();
        let mut h_small = HICON::default();
        let icon_size = match size {
            IconSize::Small => 16u32,
            IconSize::Large => 32u32,
            IconSize::Jumbo => 256u32,
        };
        let size_param = icon_size | (icon_size << 16);

        SHDefExtractIconW(
            PCWSTR(path_wide.as_ptr()),
            index,
            0,
            Some(&mut h_large),
            Some(&mut h_small),
            size_param,
        )
        .ok()?;

        let hicon = if matches!(size, IconSize::Small) && !h_small.is_invalid() {
            h_small
        } else if !h_large.is_invalid() {
            h_large
        } else if !h_small.is_invalid() {
            h_small
        } else {
            return Err("failed to extract icon resource".into());
        };

        let conversion_result =
            crate::infrastructure::windows::bitmap_conversion::hicon_to_rgba(hicon);
        let _ = DestroyIcon(hicon);
        if h_small != hicon && !h_small.is_invalid() {
            let _ = DestroyIcon(h_small);
        }
        if h_large != hicon && !h_large.is_invalid() {
            let _ = DestroyIcon(h_large);
        }

        conversion_result
    }
}

fn parse_icon_resource(resource: &str) -> Option<(String, i32)> {
    let trimmed = resource.trim().trim_matches('"');
    if trimmed.is_empty() {
        return None;
    }

    let (path, index) = if let Some(idx) = trimmed.rfind(',') {
        let path = trimmed[..idx].trim().trim_matches('"');
        let index = trimmed[idx + 1..].trim().parse::<i32>().unwrap_or(0);
        (path, index)
    } else {
        (trimmed, 0)
    };

    (!path.is_empty()).then(|| (path.to_string(), index))
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

        // For Jumbo icons, try IShellItemImageFactory first (higher quality),
        // then fall back to SHGetFileInfoW if it fails (common for USB drives).
        if matches!(size, IconSize::Jumbo) {
            let jumbo_result =
                (|| -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
                    let shell_item: IShellItem =
                        SHCreateItemFromParsingName(PCWSTR(path_wide.as_ptr()), None)?;

                    let image_factory: IShellItemImageFactory = shell_item.cast()?;

                    let size_factory = SIZE { cx: 256, cy: 256 };

                    let hbitmap: HBITMAP = image_factory.GetImage(size_factory, SIIGBF_ICONONLY)?;

                    let result =
                        crate::infrastructure::windows::bitmap_conversion::hbitmap_to_rgba(hbitmap);
                    let _ = DeleteObject(hbitmap.into());
                    result
                })();

            if let Ok(result) = jumbo_result {
                return Ok(result);
            }
            // Jumbo failed - fall through to SHGetFileInfoW below
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
        let conversion_result =
            crate::infrastructure::windows::bitmap_conversion::hicon_to_rgba(hicon);
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
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{
        SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SMALLICON,
        SHGFI_USEFILEATTRIBUTES,
    };

    // RAII guard: ensures CoUninitialize is always called to prevent COM refcount leaks.
    // Previous code called CoInitialize without CoUninitialize, leaking kernel resources
    // (Non-Paged Pool, User handles) on every call — a major source of system-wide
    // unresponsiveness after prolonged use.
    struct ComGuard(bool);
    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.0 {
                unsafe {
                    CoUninitialize();
                }
            }
        }
    }

    unsafe {
        let _com = ComGuard(CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok());

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

        // CORRECTED STRATEGY:
        // Do not use absolute paths (C:\...). Use only a simple name.
        let dummy_name = if is_folder {
            "folder".to_string()
        } else {
            let clean_ext = extension.trim_start_matches('.');
            let mapped_ext = super::canonical_icon_ext(clean_ext);
            if mapped_ext.is_empty() {
                "file".to_string()
            } else {
                format!("file.{}", mapped_ext) // ex: "file.rar"
            }
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

        // Reuse the hicon_to_rgba function we already implemented
        let conversion_result =
            crate::infrastructure::windows::bitmap_conversion::hicon_to_rgba(hicon);

        let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(hicon);

        conversion_result
    }
}
