//! Windows Shell Folder enumeration (for archive files and other virtual folders)
//! Follows .cursorrules: single responsibility, < 300 lines

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::Common::*;
use windows::Win32::UI::Shell::*;

use crate::domain::file_entry::{
    is_archive_extension, path_contains_archive_segment, FileEntry, SyncStatus, ARCHIVE_EXTENSIONS,
};

/// RAII Guard for COM initialization ( Apartment Threaded )
struct ComGuard;
impl ComGuard {
    fn new() -> Result<Option<Self>> {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr == RPC_E_CHANGED_MODE {
            return Ok(None);
        }
        if hr.is_err() {
            return Err(hr.into());
        }
        Ok(Some(Self))
    }
}
impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

/// Checks if a path should be handled via Shell Namespace (e.g. archive files)
pub fn is_shell_navigation_path(path: &Path, is_known_dir: bool) -> bool {
    let path_str = path.to_string_lossy().to_lowercase();

    // If the path is inside an archive (virtual path), always use Shell Namespace
    if path_contains_archive_segment(&path_str) {
        return true;
    }

    // If it is ALREADY known as a directory (scanned as such), it is NOT a virtual file-based folder
    if is_known_dir {
        return false;
    }

    // Archive files themselves should be handled via Shell Namespace
    if is_archive_extension(&path_str) {
        return true;
    }

    false
}

/// Lists contents of a Shell Folder (like an archive file)
pub fn list_shell_folder(path: &Path) -> Result<Vec<FileEntry>> {
    let _com = ComGuard::new()?;

    unsafe {
        // Try direct path resolution first (works for ZIP and top-level archives)
        let folder = match bind_to_shell_folder_direct(path) {
            Ok(f) => {
                log::debug!("[SHELL-FOLDER] Direct bind OK for {:?}", path);
                f
            }
            Err(e) => {
                log::warn!(
                    "[SHELL-FOLDER] Direct bind FAILED for {:?}: {:?}, trying stepwise...",
                    path, e
                );
                // Fallback: step-by-step navigation for nested archive paths
                bind_to_shell_folder_stepwise(path)?
            }
        };

        let items = enumerate_shell_children(&folder, path)?;
        log::debug!(
            "[SHELL-FOLDER] Enumerated {} items for {:?}",
            items.len(),
            path
        );
        Ok(items)
    }
}

/// Bind to IShellFolder by parsing the full path directly via SHParseDisplayName
unsafe fn bind_to_shell_folder_direct(path: &Path) -> Result<IShellFolder> {
    let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
    let path_wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    SHParseDisplayName(PCWSTR(path_wide.as_ptr()), None, &mut pidl, 0, None)?;

    let folder: IShellFolder = SHBindToObject(None, pidl, None)?;
    CoTaskMemFree(Some(pidl as *mut _));
    Ok(folder)
}

/// Fallback: navigate step-by-step through archive hierarchy.
/// 1. Find the archive root (e.g., "C:\folder\archive.7z")
/// 2. Parse the archive root via SHParseDisplayName (which works)
/// 3. Navigate into each subfolder component using IShellFolder::ParseDisplayName
unsafe fn bind_to_shell_folder_stepwise(path: &Path) -> Result<IShellFolder> {
    let path_str = path.to_string_lossy();
    let path_lower = path_str.to_lowercase();

    // Find where the archive extension ends in the path
    let archive_end = find_archive_boundary(&path_lower)
        .ok_or_else(|| Error::new(E_FAIL, "No archive root found in path"))?;

    let archive_root = &path_str[..archive_end];
    let relative = if archive_end + 1 < path_str.len() {
        &path_str[archive_end + 1..] // skip the path separator
    } else {
        return Err(Error::new(E_FAIL, "No relative path after archive root"));
    };

    // Parse archive root to get its PIDL
    let root_wide: Vec<u16> = archive_root
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut root_pidl: *mut ITEMIDLIST = std::ptr::null_mut();
    SHParseDisplayName(PCWSTR(root_wide.as_ptr()), None, &mut root_pidl, 0, None)?;

    let mut current_folder: IShellFolder = SHBindToObject(None, root_pidl, None)?;
    CoTaskMemFree(Some(root_pidl as *mut _));

    // Navigate into each subfolder component
    for component in relative.split('\\').filter(|s| !s.is_empty()) {
        let comp_wide: Vec<u16> = component.encode_utf16().chain(std::iter::once(0)).collect();
        let mut child_pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        current_folder.ParseDisplayName(
            HWND::default(),
            None,
            PWSTR(comp_wide.as_ptr() as *mut _),
            None,
            &mut child_pidl,
            std::ptr::null_mut(),
        )?;

        let next_folder: IShellFolder = current_folder.BindToObject(child_pidl, None)?;
        CoTaskMemFree(Some(child_pidl as *mut _));
        current_folder = next_folder;
    }

    Ok(current_folder)
}

/// Finds the position right after the archive extension in a lowercased path.
/// For "c:\folder\archive.7z\subfolder" returns the position after ".7z".
fn find_archive_boundary(path_lower: &str) -> Option<usize> {
    let mut best_end: Option<usize> = None;

    for ext in ARCHIVE_EXTENSIONS {
        for sep in &["\\", "/"] {
            let pattern = format!("{}{}", ext, sep);
            if let Some(pos) = path_lower.find(&pattern) {
                let end = pos + ext.len();
                // Take the earliest (outermost) archive boundary
                match best_end {
                    None => best_end = Some(end),
                    Some(prev) if end < prev => best_end = Some(end),
                    _ => {}
                }
            }
        }
    }

    best_end
}

/// Enumerate children of an already-bound IShellFolder
unsafe fn enumerate_shell_children(
    folder: &IShellFolder,
    parent_path: &Path,
) -> Result<Vec<FileEntry>> {
    let mut items = Vec::new();

    let mut enum_id_list: Option<IEnumIDList> = None;
    let flags = SHCONTF_FOLDERS.0 | SHCONTF_NONFOLDERS.0 | SHCONTF_INCLUDEHIDDEN.0;
    folder
        .EnumObjects(HWND::default(), flags as u32, &mut enum_id_list)
        .ok()?;

    if let Some(enumerator) = enum_id_list {
        let mut item_pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        let mut fetched: u32 = 0;

        while enumerator
            .Next(std::slice::from_mut(&mut item_pidl), Some(&mut fetched))
            .is_ok()
            && fetched > 0
        {
            if let Ok(item_entry) = process_shell_child(folder, item_pidl, parent_path) {
                items.push(item_entry);
            }
            CoTaskMemFree(Some(item_pidl as *mut _));
        }
    }

    Ok(items)
}

/// Processes a single child PIDL from a parent IShellFolder
unsafe fn process_shell_child(
    parent: &IShellFolder,
    child_pidl: *mut ITEMIDLIST,
    parent_path: &Path,
) -> Result<FileEntry> {
    // Get display name
    let mut strret = STRRET::default();
    parent.GetDisplayNameOf(child_pidl, SHGDN_INFOLDER, &mut strret)?;

    let mut name_buf = [0u16; 260];
    StrRetToBufW(&mut strret, Some(child_pidl), &mut name_buf)?;
    let name = PCWSTR::from_raw(name_buf.as_ptr())
        .to_string()
        .unwrap_or_default();

    // Bind to IShellItem to get larger metadata
    let item: IShellItem = SHCreateShellItem(None, Some(parent), child_pidl)?;
    let item2: IShellItem2 = item.cast()?;

    // Check folder attributes
    // IMPORTANT: GetAttributesOf uses the input value as a MASK of which attributes to query.
    // We MUST set the bits we want to check, otherwise the shell extension may return 0.
    let mut attributes = 0x20400000u32; // SFGAO_FOLDER (0x20000000) | SFGAO_STREAM (0x00400000)
    parent.GetAttributesOf(&[child_pidl as *const _], &mut attributes)?;

    // SFGAO_FOLDER (0x20000000) - Primary folder check
    // SFGAO_STREAM (0x00400000) - Stream (file-like, not a pure folder)
    let sfgao_folder = (attributes & 0x20000000) != 0;
    let sfgao_stream = (attributes & 0x00400000) != 0;
    // A folder that is also a stream (e.g., ZIP inside archive) is treated as a directory.
    // A pure folder (SFGAO_FOLDER without SFGAO_STREAM) is always a directory.
    // For non-ZIP archives, Windows may not set SFGAO_FOLDER for subfolders,
    // so we also check if size == 0 and no extension as a heuristic.
    let is_dir = sfgao_folder;

    log::trace!(
        "[SHELL-CHILD] name={:?}, attributes=0x{:08X}, FOLDER={}, STREAM={}, is_dir={}",
        name, attributes, sfgao_folder, sfgao_stream, is_dir
    );

    // Size (System.Size)
    let size: u64 = item2
        .GetUInt64(&crate::infrastructure::windows::recycle_bin::PKEY_SIZE)
        .unwrap_or_default();

    // Modified Date (System.DateModified)
    let pkey_date_modified: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0xB725F130_47EF_101A_A5F1_02608C9EEBAC),
        pid: 14,
    };

    let modified = match item2.GetFileTime(&pkey_date_modified) {
        Ok(ft) => {
            let windows_ticks = ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
            if windows_ticks > 116444736000000000 {
                (windows_ticks - 116444736000000000) / 10_000_000
            } else {
                0
            }
        }
        Err(_) => 0,
    };

    let path = parent_path.join(&name);

    Ok(FileEntry {
        path,
        name,
        is_dir,
        size,
        modified,
        folder_cover: None,
        drive_info: None,
        sync_status: SyncStatus::None,
        is_hidden: false,
        deletion_date: None,
        recycle_original_path: None,
    })
}
