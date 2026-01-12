//! Windows Recycle Bin implementation
//! Uses IShellItem2 to retrieve robust metadata (original path, deletion date)

use std::path::PathBuf;
use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::System::Com::*,
    Win32::UI::Shell::*,
    Win32::UI::Shell::PropertiesSystem::*,
    Win32::System::Com::StructuredStorage::*,
};

// Property keys for Recycle Bin items
pub const PKEY_SIZE: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xB725F130_47EF_101A_A5F1_02608C9EEBAC),
    pid: 12,
};
pub const PKEY_RECYCLE_ORIGINAL_LOCATION: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9B174B33_0711_4E65_8F2D_36015C466DB5),
    pid: 2,
};
pub const PKEY_RECYCLE_DATE_DELETED: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9B174B33_0711_4E65_8F2D_36015C466DB5),
    pid: 3,
};
pub const PKEY_ITEMNAMEDDISPLAY: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xB725F130_47EF_101A_A5F1_02608C9EEBAC),
    pid: 10,
};

/// Represents an item in the Recycle Bin
#[derive(Debug, Clone)]
pub struct RecycleBinItem {
    pub name: String,
    pub original_path: PathBuf,
    pub date_deleted: String,
    pub size: u64,
    pub is_directory: bool,
}

/// Retrieves the total count and size of items in the Recycle Bin
pub fn get_recycle_bin_info() -> Result<(u64, u64)> {
    unsafe {
        let mut info = SHQUERYRBINFO {
            cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
            ..Default::default()
        };
        
        SHQueryRecycleBinW(PCWSTR::null(), &mut info)?;
        Ok((info.i64NumItems as u64, info.i64Size as u64))
    }
}

/// Enumerates all items currently in the Recycle Bin using IShellItem2 API
pub fn enumerate_recycle_bin() -> Result<Vec<RecycleBinItem>> {
    unsafe {
        eprintln!("[Lixeira] Starting enumeration...");
        
        let mut items = Vec::new();

        // Initialize COM
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        // Get Recycle Bin folder using known folder ID
        let recycle_bin_folder: IShellItem = SHGetKnownFolderItem(
            &FOLDERID_RecycleBinFolder,
            KF_FLAG_DEFAULT,
            None,
        )?;

        // Get IEnumShellItems to enumerate children
        let enum_items: IEnumShellItems = recycle_bin_folder.BindToHandler(None, &BHID_EnumItems)?;

        loop {
            let mut shell_items: [Option<IShellItem>; 1] = [None];
            let mut fetched: u32 = 0;
            
            let hr = enum_items.Next(&mut shell_items, Some(&mut fetched));
            
            if hr.is_err() || fetched == 0 {
                break;
            }

            if let Some(shell_item) = shell_items[0].take() {
                // Cast to IShellItem2 for property access
                let shell_item2: IShellItem2 = match shell_item.cast() {
                    Ok(item) => item,
                    Err(_) => continue,
                };

                // Get display name (just the file/folder name, not full path)
                let name = get_shell_item_name(&shell_item2);
                
                // Get original location (folder where item was before deletion)
                let original_location = get_shell_item_string_property(&shell_item2, &PKEY_RECYCLE_ORIGINAL_LOCATION)
                    .unwrap_or_default();
                
                // Build the original full path
                let original_path = if !original_location.is_empty() {
                    PathBuf::from(&original_location).join(&name)
                } else {
                    PathBuf::from(&name)
                };
                
                // Get date deleted
                let date_deleted = get_shell_item_string_property(&shell_item2, &PKEY_RECYCLE_DATE_DELETED)
                    .unwrap_or_default();
                
                // Get size
                let size = get_shell_item_u64_property(&shell_item2, &PKEY_SIZE).unwrap_or(0);
                
                // Check if it's a folder
                const PKEY_FILE_ATTRIBUTES: PROPERTYKEY = PROPERTYKEY {
                    fmtid: GUID::from_u128(0xB725F130_47EF_101A_A5F1_02608C9EEBAC),
                    pid: 13,
                };
                let is_directory = get_shell_item_u64_property(&shell_item2, &PKEY_FILE_ATTRIBUTES)
                    .map(|attrs| (attrs & 0x10) != 0)
                    .unwrap_or(false);

                items.push(RecycleBinItem {
                    name,
                    original_path,
                    date_deleted,
                    size,
                    is_directory,
                });
            }
        }

        eprintln!("[Lixeira] Enumeration complete. Total items: {}", items.len());
        Ok(items)
    }
}

/// Get display name from IShellItem2 (the friendly name shown in Explorer)
unsafe fn get_shell_item_name(item: &IShellItem2) -> String {
    // SIGDN_NORMALDISPLAY gives the friendly name as shown in Explorer
    // For Recycle Bin items, this is the ORIGINAL filename, not the $Rxxxxxx name
    if let Ok(name_ptr) = item.GetDisplayName(SIGDN_NORMALDISPLAY) {
        let name = name_ptr.to_string().unwrap_or_default();
        CoTaskMemFree(Some(name_ptr.as_ptr() as *mut _));
        if !name.is_empty() {
            return name;
        }
    }
    
    // Fallback to PKEY_ItemNameDisplay
    if let Ok(name) = get_shell_item_string_property(item, &PKEY_ITEMNAMEDDISPLAY) {
        if !name.is_empty() {
            return name;
        }
    }
    
    "Item".to_string()
}

/// Get string property from IShellItem2
unsafe fn get_shell_item_string_property(item: &IShellItem2, pkey: &PROPERTYKEY) -> Result<String> {
    let prop_var = item.GetProperty(pkey)?;
    let pv_ptr: *const PROPVARIANT = &prop_var as *const _ as *const _;
    
    let str_ptr = PropVariantToStringAlloc(pv_ptr)?;
    let result = str_ptr.to_string().map_err(|_| Error::from_win32())?;
    CoTaskMemFree(Some(str_ptr.as_ptr() as *mut _));
    
    Ok(result)
}

/// Get u64 property from IShellItem2
unsafe fn get_shell_item_u64_property(item: &IShellItem2, pkey: &PROPERTYKEY) -> Result<u64> {
    let prop_var = item.GetProperty(pkey)?;
    let pv_ptr: *const PROPVARIANT = &prop_var as *const _ as *const _;
    
    let val = PropVariantToUInt64(pv_ptr)?;
    Ok(val)
}
