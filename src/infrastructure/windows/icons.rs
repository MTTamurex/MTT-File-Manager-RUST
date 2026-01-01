//! Windows icon extraction functions
//! Follows .cursorrules: single responsibility, < 300 lines

use std::path::{Path, PathBuf};
use windows::{
    Win32::Foundation::*,
    Win32::Graphics::Gdi::*,
    Win32::System::Com::*,
    Win32::Storage::FileSystem::*,
    Win32::UI::Shell::*,
    Win32::UI::WindowsAndMessaging::*,
    core::*,
};

use crate::domain::file_entry::IconSize;

/// Extracts the "This PC" (My Computer) icon using PIDL (robust method).
///
/// # Safety
/// Uses SHGetSpecialFolderLocation and SHGetFileInfoW with PIDL.
/// Always frees the PIDL with CoTaskMemFree.
pub fn extract_computer_icon(size: IconSize) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        // 1. Get PIDL for "My Computer" (CSIDL_DRIVES)
        let pidl = match SHGetSpecialFolderLocation(HWND(std::ptr::null_mut()), CSIDL_DRIVES as i32) {
            Ok(p) => p,
            Err(_) => {
                return Err("Failed to get PIDL for My Computer".into());
            }
        };
        
        let mut shfi = SHFILEINFOW::default();
        
        // 2. Flags with SHGFI_PIDL (CRITICAL!)
        let flags = SHGFI_PIDL | SHGFI_ICON | match size {
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
pub fn extract_thumbnail(path: &PathBuf) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        let path_str = path.to_string_lossy().to_string();
        let path_wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();
        
        let shell_item: IShellItem = SHCreateItemFromParsingName(
            PCWSTR(path_wide.as_ptr()),
            None,
        )?;
        
        let image_factory: IShellItemImageFactory = shell_item.cast()?;
        
        let size = SIZE {
            cx: 256,
            cy: 256,
        };
        let hbitmap: HBITMAP = image_factory.GetImage(size, SIIGBF_THUMBNAILONLY)?;
        
        let (rgba_data, width, height) = super::bitmap_conversion::hbitmap_to_rgba(hbitmap)?;
        
        let _ = DeleteObject(hbitmap);
        
        Ok((rgba_data, width, height))
    }
}

/// Extracts the native Windows icon for a file extension.
///
/// Uses FILE_ATTRIBUTE_NORMAL + SHGFI_USEFILEATTRIBUTES to get the default icon for the type.
///
/// # Safety
/// Uses SHGetFileInfoW with dummy path. HICON is always freed with DestroyIcon.
pub fn extract_file_icon(
    extension: &str,  // ".pdf", ".exe", etc.
    size: IconSize,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        // Create dummy path with extension (e.g., "dummy.pdf")
        let dummy_path = format!("dummy{}", extension);
        let path_wide: Vec<u16> = dummy_path
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        
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
pub fn extract_folder_icon(size: IconSize) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
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
        let path_wide: Vec<u16> = path.to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        
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
            let shell_item: IShellItem = SHCreateItemFromParsingName(
                PCWSTR(path_wide.as_ptr()),
                None,
            )?;
            
            let image_factory: IShellItemImageFactory = shell_item.cast()?;
            
            let size_factory = SIZE {
                cx: 256,
                cy: 256,
            };
            
            // SIIGBF_ICONONLY to ensure we get the icon and not a thumbnail if it were a file
            let hbitmap: HBITMAP = image_factory.GetImage(size_factory, SIIGBF_ICONONLY)?;
            
            let (rgba_data, width, height) = super::bitmap_conversion::hbitmap_to_rgba(hbitmap)?;
            let _ = DeleteObject(hbitmap);
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
