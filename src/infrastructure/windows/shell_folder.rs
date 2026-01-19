//! Windows Shell Folder enumeration (for ZIP files and other virtual folders)
//! Follows .cursorrules: single responsibility, < 300 lines

use std::path::Path;
use windows::core::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::Shell::Common::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::PropertiesSystem::*;
use windows::Win32::Foundation::*;
use std::os::windows::ffi::OsStrExt;

use crate::domain::file_entry::{FileEntry, SyncStatus};

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

/// Checks if a path should be handled via Shell Namespace (e.g. ZIP files)
pub fn is_shell_navigation_path(path: &Path) -> bool {
    let path_str = path.to_string_lossy().to_lowercase();
    if path_str.contains(".zip") {
        // If it's a physical directory, it's not a virtual ZIP
        if crate::infrastructure::windows::file_system::is_directory(path) {
            return false;
        }
        return true;
    }
    false
}

/// Lists contents of a Shell Folder (like a ZIP file)
pub fn list_shell_folder(path: &Path) -> Result<Vec<FileEntry>> {
    let _com = ComGuard::new()?;
    
    unsafe {
        // 1. Convert path to PIDL
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        SHParseDisplayName(PCWSTR(path_wide.as_ptr()), None, &mut pidl, 0, None)?;
        
        let mut items = Vec::new();
        
        // 2. Bind to IShellFolder
        let folder: IShellFolder = SHBindToObject(None, pidl, None)?;
        
        // 3. Enumerate children
        let mut enum_id_list: Option<IEnumIDList> = None;
        let flags = SHCONTF_FOLDERS.0 | SHCONTF_NONFOLDERS.0 | SHCONTF_INCLUDEHIDDEN.0;
        folder.EnumObjects(None, flags as u32, &mut enum_id_list).ok()?;
        
        if let Some(enumerator) = enum_id_list {
            let mut item_pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            let mut fetched: u32 = 0;
            
            while enumerator.Next(std::slice::from_mut(&mut item_pidl), Some(&mut fetched)).is_ok() && fetched > 0 {
                if let Ok(item_entry) = process_shell_child(&folder, item_pidl, path) {
                    items.push(item_entry);
                }
                CoTaskMemFree(Some(item_pidl as *mut _));
            }
        }
        
        CoTaskMemFree(Some(pidl as *mut _));
        Ok(items)
    }
}

/// Processes a single child PIDL from a parent IShellFolder
unsafe fn process_shell_child(parent: &IShellFolder, child_pidl: *mut ITEMIDLIST, parent_path: &Path) -> Result<FileEntry> {
    // Get display name
    let mut strret = STRRET::default();
    parent.GetDisplayNameOf(child_pidl, SHGDN_INFOLDER, &mut strret)?;
    
    let mut name_buf = [0u16; 260];
    StrRetToBufW(&mut strret, Some(child_pidl), &mut name_buf)?;
    let name = PCWSTR::from_raw(name_buf.as_ptr()).to_string().unwrap_or_default();
    
    // Bind to IShellItem to get larger metadata
    let item: IShellItem = SHCreateShellItem(None, Some(parent), child_pidl)?;
    let item2: IShellItem2 = item.cast()?;
    
    let mut attributes = 0x20000000u32; // SFGAO_FOLDER
    parent.GetAttributesOf(&[child_pidl as *const _], &mut attributes)?;
    let is_dir = (attributes & 0x20000000) != 0;
    
    // Size (System.Size)
    let size = match item2.GetUInt64(&crate::infrastructure::windows::recycle_bin::PKEY_SIZE) {
        Ok(s) => s,
        Err(_) => 0,
    };
    
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
        },
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
        deletion_date: None,
    })
}
