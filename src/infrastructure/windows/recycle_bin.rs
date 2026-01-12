//! Windows Recycle Bin implementation
//! Uses IShellItem2 to retrieve robust metadata (original path, deletion date)

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
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
// System.Recycle.DeletedFrom - the original folder location
pub const PKEY_RECYCLE_DELETED_FROM: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9B174B33_40FF_11D2_A27E_00C04FC30871),
    pid: 2,
};
// System.Recycle.DateDeleted
pub const PKEY_RECYCLE_DATE_DELETED: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9B174B33_40FF_11D2_A27E_00C04FC30871),
    pid: 3,
};
// System.ItemNameDisplay - the display name
pub const PKEY_ITEMNAMEDISPLAY: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xB725F130_47EF_101A_A5F1_02608C9EEBAC),
    pid: 10,
};

/// Represents an item in the Recycle Bin
#[derive(Debug, Clone)]
pub struct RecycleBinItem {
    /// Display name (filename only, e.g., "document.docx")
    pub name: String,
    /// Parent folder where the item was deleted from (e.g., "C:\Users\Documents")
    pub parent_folder: String,
    /// Full original path for restoration (e.g., "C:\Users\Documents\document.docx")
    pub original_path: PathBuf,
    /// Date when item was deleted
    pub date_deleted: String,
    /// Size in bytes
    pub size: u64,
    /// Whether item is a directory
    pub is_directory: bool,
    /// File extension for icon lookup (e.g., ".docx")
    pub extension: String,
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

/// Enumerates recycle bin items in batches, sending them via channel for progressive loading
pub fn enumerate_recycle_bin_streaming(
    sender: Sender<Vec<RecycleBinItem>>,
    generation: Arc<AtomicUsize>,
    my_gen: usize,
    batch_size: usize,
) {
    unsafe {
        eprintln!("[Lixeira] Starting streaming enumeration...");
        
        // Initialize COM for this thread
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        // Get Recycle Bin folder
        let recycle_bin_folder: IShellItem = match SHGetKnownFolderItem(
            &FOLDERID_RecycleBinFolder,
            KF_FLAG_DEFAULT,
            None,
        ) {
            Ok(folder) => folder,
            Err(e) => {
                eprintln!("[Lixeira] Failed to get recycle bin folder: {:?}", e);
                let _ = sender.send(Vec::new()); // Signal completion
                return;
            }
        };

        // Get enumerator
        let enum_items: IEnumShellItems = match recycle_bin_folder.BindToHandler(None, &BHID_EnumItems) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[Lixeira] Failed to get enumerator: {:?}", e);
                let _ = sender.send(Vec::new());
                return;
            }
        };

        let mut batch = Vec::with_capacity(batch_size);
        let mut total_count = 0;

        loop {
            // Check if cancelled
            if generation.load(Ordering::Relaxed) != my_gen {
                eprintln!("[Lixeira] Enumeration cancelled");
                return;
            }

            let mut shell_items: [Option<IShellItem>; 1] = [None];
            let mut fetched: u32 = 0;
            
            if enum_items.Next(&mut shell_items, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }

            if let Some(shell_item) = shell_items[0].take() {
                if let Some(item) = process_shell_item(&shell_item) {
                    batch.push(item);
                    total_count += 1;

                    // Send batch when full
                    if batch.len() >= batch_size {
                        if generation.load(Ordering::Relaxed) != my_gen {
                            return;
                        }
                        let _ = sender.send(std::mem::take(&mut batch));
                        batch = Vec::with_capacity(batch_size);
                    }
                }
            }
        }

        // Send remaining items
        if !batch.is_empty() && generation.load(Ordering::Relaxed) == my_gen {
            let _ = sender.send(batch);
        }

        // Signal completion with empty batch
        let _ = sender.send(Vec::new());

        eprintln!("[Lixeira] Enumeration complete. Total items: {}", total_count);
    }
}

/// Process a single shell item into RecycleBinItem
unsafe fn process_shell_item(shell_item: &IShellItem) -> Option<RecycleBinItem> {
    let shell_item2: IShellItem2 = shell_item.cast().ok()?;

    // Get display name - this should be the original filename
    let name = get_item_display_name(&shell_item2);
    
    // Get parent folder (where item was deleted from)
    let parent_folder = get_shell_item_string_property(&shell_item2, &PKEY_RECYCLE_DELETED_FROM)
        .unwrap_or_default();
    
    // Build full original path
    let original_path = if !parent_folder.is_empty() {
        PathBuf::from(&parent_folder).join(&name)
    } else {
        PathBuf::from(&name)
    };
    
    // Get extension for icon lookup
    let extension = std::path::Path::new(&name)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    
    // Get date deleted
    let date_deleted = get_shell_item_string_property(&shell_item2, &PKEY_RECYCLE_DATE_DELETED)
        .unwrap_or_default();
    
    // Get size
    let size = get_shell_item_u64_property(&shell_item2, &PKEY_SIZE).unwrap_or(0);
    
    // Check if directory
    const PKEY_FILE_ATTRIBUTES: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0xB725F130_47EF_101A_A5F1_02608C9EEBAC),
        pid: 13,
    };
    let is_directory = get_shell_item_u64_property(&shell_item2, &PKEY_FILE_ATTRIBUTES)
        .map(|attrs| (attrs & 0x10) != 0)
        .unwrap_or(false);

    Some(RecycleBinItem {
        name,
        parent_folder,
        original_path,
        date_deleted,
        size,
        is_directory,
        extension,
    })
}

/// Get the display name of an item (original filename)
unsafe fn get_item_display_name(item: &IShellItem2) -> String {
    // Try PKEY_ItemNameDisplay first - this gives the original filename
    if let Ok(name) = get_shell_item_string_property(item, &PKEY_ITEMNAMEDISPLAY) {
        if !name.is_empty() && !name.starts_with("$R") && !name.contains("\\$Recycle") {
            return name;
        }
    }
    
    // Try SIGDN_NORMALDISPLAY
    if let Ok(name_ptr) = item.GetDisplayName(SIGDN_NORMALDISPLAY) {
        let name = name_ptr.to_string().unwrap_or_default();
        CoTaskMemFree(Some(name_ptr.as_ptr() as *mut _));
        if !name.is_empty() && !name.starts_with("$R") && !name.contains("\\$Recycle") {
            return name;
        }
    }
    
    // Try SIGDN_PARENTRELATIVEEDITING - sometimes has better name
    if let Ok(name_ptr) = item.GetDisplayName(SIGDN_PARENTRELATIVEEDITING) {
        let name = name_ptr.to_string().unwrap_or_default();
        CoTaskMemFree(Some(name_ptr.as_ptr() as *mut _));
        if !name.is_empty() && !name.starts_with("$R") {
            return name;
        }
    }
    
    // Try SIGDN_PARENTRELATIVEFORADDRESSBAR as last resort
    if let Ok(name_ptr) = item.GetDisplayName(SIGDN_PARENTRELATIVEFORADDRESSBAR) {
        let name = name_ptr.to_string().unwrap_or_default();
        CoTaskMemFree(Some(name_ptr.as_ptr() as *mut _));
        // Extract just filename from any path-like result
        if !name.is_empty() {
            if let Some(filename) = name.rsplit(['\\', '/']).next() {
                if !filename.starts_with("$R") {
                    return filename.to_string();
                }
            }
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

/// Legacy function for backwards compatibility - enumerates all at once
pub fn enumerate_recycle_bin() -> Result<Vec<RecycleBinItem>> {
    unsafe {
        eprintln!("[Lixeira] Starting enumeration...");
        
        let mut items = Vec::new();
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let recycle_bin_folder: IShellItem = SHGetKnownFolderItem(
            &FOLDERID_RecycleBinFolder,
            KF_FLAG_DEFAULT,
            None,
        )?;

        let enum_items: IEnumShellItems = recycle_bin_folder.BindToHandler(None, &BHID_EnumItems)?;

        loop {
            let mut shell_items: [Option<IShellItem>; 1] = [None];
            let mut fetched: u32 = 0;
            
            if enum_items.Next(&mut shell_items, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }

            if let Some(shell_item) = shell_items[0].take() {
                if let Some(item) = process_shell_item(&shell_item) {
                    items.push(item);
                }
            }
        }

        eprintln!("[Lixeira] Enumeration complete. Total items: {}", items.len());
        Ok(items)
    }
}
