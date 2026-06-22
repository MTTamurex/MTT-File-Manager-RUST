use crate::domain::file_entry::IconSize;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::{
    core::*, Win32::Foundation::*, Win32::Graphics::Gdi::*, Win32::Storage::FileSystem::*,
    Win32::System::Com::*, Win32::UI::Shell::Common::*, Win32::UI::Shell::*,
    Win32::UI::WindowsAndMessaging::*,
};

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
        let pidl = match SHGetSpecialFolderLocation(Some(HWND::default()), CSIDL_DRIVES as i32) {
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
        let conversion_result =
            crate::infrastructure::windows::bitmap_conversion::hicon_to_rgba(hicon);

        let _ = DestroyIcon(hicon);

        conversion_result
    }
}

/// Extracts the Recycle Bin icon using SHGetKnownFolderIDList
pub fn extract_recycle_bin_icon(
    size: IconSize,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    use windows::Win32::UI::Shell::{
        FOLDERID_RecycleBinFolder, IShellItem, IShellItemImageFactory, SHGetFileInfoW,
        SHGetKnownFolderIDList, SHGetKnownFolderItem, KF_FLAG_DEFAULT, SHFILEINFOW, SHGFI_ICON,
        SHGFI_LARGEICON, SHGFI_PIDL, SHGFI_SMALLICON, SIIGBF_ICONONLY,
    };

    unsafe {
        if matches!(size, IconSize::Jumbo) {
            let _com = crate::infrastructure::windows::recycle_bin::ComApartmentGuard::init_sta_best_effort();

            if let Ok(shell_item) = SHGetKnownFolderItem::<IShellItem>(
                &FOLDERID_RecycleBinFolder,
                KF_FLAG_DEFAULT,
                None,
            ) {
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
        let conversion_result =
            crate::infrastructure::windows::bitmap_conversion::hicon_to_rgba(hicon);
        let _ = DestroyIcon(hicon);

        conversion_result
    }
}

/// Extracts icon using Shell Namespace (PIDL), supporting virtual paths (ZIP contents).
///
/// This resolves the path to an ITEMIDLIST (PIDL) using standard Shell parsing,
/// allowing icons to be retrieved for items inside ZIPs.
pub fn extract_shell_icon(
    path: &Path,
    size: IconSize,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        let path_wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // For Jumbo, try IShellItemImageFactory first (256x256, high quality).
        // SHCreateItemFromParsingName supports virtual paths (ZIP contents)
        // just like SHParseDisplayName.
        if matches!(size, IconSize::Jumbo) {
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
            // Fall through to PIDL-based extraction if ImageFactory fails.
        }

        // 1. Parse Path to PIDL
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();

        // SHParseDisplayName handles virtual paths like "C:\Archive.zip\Folder"
        if SHParseDisplayName(PCWSTR(path_wide.as_ptr()), None, &mut pidl, 0, None).is_err() {
            return Err("Failed to parse shell path".into());
        }

        let mut shfi = SHFILEINFOW::default();

        // 2. Use SHGFI_PIDL flag to query by PIDL instead of path string
        let flags = SHGFI_PIDL
            | SHGFI_ICON
            | match size {
                IconSize::Small => SHGFI_SMALLICON,
                IconSize::Large | IconSize::Jumbo => SHGFI_LARGEICON,
            };

        let result = SHGetFileInfoW(
            PCWSTR(pidl as *const u16),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        );

        // 3. Always Free PIDL
        CoTaskMemFree(Some(pidl as *const std::ffi::c_void));

        if result == 0 || shfi.hIcon.is_invalid() {
            return Err("Failed to get shell icon".into());
        }

        let hicon = shfi.hIcon;
        let conversion_result =
            crate::infrastructure::windows::bitmap_conversion::hicon_to_rgba(hicon);
        let _ = DestroyIcon(hicon);

        conversion_result
    }
}
