//! Windows Recycle Bin implementation
//! Uses IShellItem2 to retrieve robust metadata (original path, deletion date)

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use windows::core::Interface;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::Common::*;
use windows::Win32::UI::Shell::*;

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
    /// Physical path in $Recycle.Bin (e.g., "C:\$Recycle.Bin\...\$R123456.docx")
    pub physical_path: PathBuf,
    /// Date when item was deleted
    pub date_deleted: String,
    /// Deletion timestamp in UNIX seconds (0 when unavailable)
    pub date_deleted_unix: u64,
    /// Size in bytes
    pub size: u64,
    /// Whether item is a directory
    pub is_directory: bool,
    /// File extension for icon lookup (e.g., ".docx")
    pub extension: String,
}

/// RAII guard for COM apartment initialization on the current thread.
struct ComApartmentGuard {
    initialized: bool,
}

impl ComApartmentGuard {
    fn init_sta_best_effort() -> Self {
        let initialized = unsafe {
            // SAFETY: Initializes COM apartment for the current thread.
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok()
        };
        Self { initialized }
    }
}

impl Drop for ComApartmentGuard {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                // SAFETY: Balanced with successful CoInitializeEx in init_sta_best_effort.
                CoUninitialize();
            }
        }
    }
}

/// Retrieves the total count and size of items in the Recycle Bin
pub fn get_recycle_bin_info() -> Result<(u64, u64)> {
    unsafe {
        let mut info = SHQUERYRBINFO {
            cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
            ..Default::default()
        };

        SHQueryRecycleBinW(PCWSTR::default(), &mut info)?;
        Ok((info.i64NumItems as u64, info.i64Size as u64))
    }
}

pub fn enumerate_recycle_bin_streaming(
    sender: Sender<Vec<RecycleBinItem>>,
    generation: Arc<AtomicUsize>,
    my_gen: usize,
    batch_size: usize,
) {
    unsafe {
        eprintln!("[Lixeira] Starting streaming enumeration (Shell API)...");

        let _com = ComApartmentGuard::init_sta_best_effort();

        // 1. Get Desktop Folder
        let desktop: IShellFolder = match SHGetDesktopFolder() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[Lixeira] Failed to get desktop folder: {:?}", e);
                let _ = sender.send(Vec::new());
                return;
            }
        };

        // 2. Get Recycle Bin PIDL
        let recycle_bin_pidl = match SHGetKnownFolderIDList(&FOLDERID_RecycleBinFolder, 0, None) {
            Ok(pidl) => pidl,
            Err(e) => {
                eprintln!("[Lixeira] Failed to get Recycle Bin PIDL: {:?}", e);
                let _ = sender.send(Vec::new());
                return;
            }
        };

        // 3. Bind to Recycle Bin Folder (IShellFolder)
        let recycle_bin_folder: IShellFolder = match desktop.BindToObject(recycle_bin_pidl, None) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[Lixeira] Failed to bind to Recycle Bin folder: {:?}", e);
                CoTaskMemFree(Some(recycle_bin_pidl as *mut _));
                let _ = sender.send(Vec::new());
                return;
            }
        };
        CoTaskMemFree(Some(recycle_bin_pidl as *mut _)); // Release PIDL

        // 4. Enumerate Objects (as PIDLs)
        // Fix SHCONTF flags: cast to u32 because EnumObjects takes u32
        let flags = (SHCONTF_FOLDERS.0 | SHCONTF_NONFOLDERS.0 | SHCONTF_INCLUDEHIDDEN.0) as u32;
        let mut enum_list_opt: Option<IEnumIDList> = None;

        if recycle_bin_folder
            .EnumObjects(
                windows::Win32::Foundation::HWND::default(),
                flags,
                &mut enum_list_opt,
            )
            .is_err()
        {
            eprintln!("[Lixeira] Failed to get enumerator");
            let _ = sender.send(Vec::new());
            return;
        }
        let enum_list = enum_list_opt.unwrap();

        let mut batch = Vec::with_capacity(batch_size);
        let mut total_count = 0;

        loop {
            // Check cancellation
            if generation.load(Ordering::Relaxed) != my_gen {
                return;
            }

            let mut fetched: u32 = 0;
            let mut pidl_child: *mut ITEMIDLIST = std::ptr::null_mut();

            // Next expects a slice of pointers. We want 1 item.
            if enum_list
                .Next(std::slice::from_mut(&mut pidl_child), Some(&mut fetched))
                .is_err()
                || fetched == 0
            {
                break;
            }

            // --- PROCESS SINGLE ITEM ---
            // Create IShellItem from PIDL (child).
            // SHCreateShellItem logic: pidlParent=None, psfParent=Some(folder), pidl=child
            if let Ok(shell_item) = SHCreateShellItem(None, Some(&recycle_bin_folder), pidl_child) {
                // Get deletion date using the Shell Folder API + PIDL
                let date_deleted = get_date_deleted_from_pidl(&recycle_bin_folder, pidl_child);

                // Process other properties using existing helper
                if let Some(mut item) = process_shell_item(&shell_item) {
                    // Overwrite with the date obtained from column view
                    item.date_deleted = date_deleted;

                    batch.push(item);
                    total_count += 1;
                }
            }

            // Validate PIDL release
            CoTaskMemFree(Some(pidl_child as *mut _));

            // Send batch
            if batch.len() >= batch_size {
                if generation.load(Ordering::Relaxed) != my_gen {
                    return;
                }
                let _ = sender.send(std::mem::take(&mut batch));
                batch = Vec::with_capacity(batch_size);
            }
        }

        if !batch.is_empty() && generation.load(Ordering::Relaxed) == my_gen {
            let _ = sender.send(batch);
        }

        // Completion signal
        let _ = sender.send(Vec::new());
        eprintln!(
            "[Lixeira] Enumeration complete (Shell API). Total items: {}",
            total_count
        );
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

    // Get physical path (Parsing path) - this gives us the $R path in $Recycle.Bin
    let physical_path =
        if let Ok(name_ptr) = shell_item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING) {
            let path_str = name_ptr.to_string().unwrap_or_default();
            CoTaskMemFree(Some(name_ptr.as_ptr() as *mut _));
            PathBuf::from(path_str)
        } else {
            PathBuf::new()
        };

    // Get extension for icon lookup
    let extension = std::path::Path::new(&name)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        .unwrap_or_default();

    // Get date deleted as both UNIX timestamp and formatted string.
    let (date_deleted_unix, date_deleted) =
        get_shell_item_filetime_property(&shell_item2, &PKEY_RECYCLE_DATE_DELETED)
            .unwrap_or_else(|_| (0, "Desconhecido".to_string()));

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
        physical_path,
        date_deleted,
        date_deleted_unix,
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
    let str_ptr = item.GetString(pkey)?;
    let result = str_ptr.to_string().map_err(|_| Error::from_win32())?;
    CoTaskMemFree(Some(str_ptr.0 as *mut _));
    Ok(result)
}

/// Get u64 property from IShellItem2
unsafe fn get_shell_item_u64_property(item: &IShellItem2, pkey: &PROPERTYKEY) -> Result<u64> {
    item.GetUInt64(pkey)
}

/// Get FILETIME property from IShellItem2 and format as date string
unsafe fn get_shell_item_filetime_property(
    item: &IShellItem2,
    pkey: &PROPERTYKEY,
) -> Result<(u64, String)> {
    if let Ok(str_ptr) = item.GetString(pkey) {
        let result = str_ptr.to_string().unwrap_or_default();
        CoTaskMemFree(Some(str_ptr.0 as *mut _));
        if !result.is_empty() {
            // Format: '2026/01/12:19:21:00.000' -> '12/01/2026 19:21'
            let formatted = format_recycle_date(&result);
            // Try to also read FILETIME for stable numeric sorting.
            if let Ok(ft) = item.GetFileTime(pkey) {
                let ft_val = ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
                if let Some(unix_secs) = filetime_to_unix_secs(ft_val) {
                    return Ok((unix_secs, formatted));
                }
            }
            return Ok((0, formatted));
        }
    }

    // Fallback: Try PropVariantToUInt64 for raw FILETIME
    if let Ok(ft) = item.GetFileTime(pkey) {
        // Fallback: raw FILETIME
        let ft_val = ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
        if let Some(unix_secs) = filetime_to_unix_secs(ft_val) {
            return Ok((unix_secs, super::formatting::format_date(unix_secs)));
        }
    }

    Err(Error::from_win32())
}

#[inline]
fn filetime_to_unix_secs(filetime: u64) -> Option<u64> {
    if filetime == 0 {
        return None;
    }
    const FILETIME_TO_UNIX: u64 = 116444736000000000;
    if filetime <= FILETIME_TO_UNIX {
        return None;
    }
    Some((filetime - FILETIME_TO_UNIX) / 10_000_000)
}

/// Format recycle bin date from '2026/01/12:19:21:00.000' to '12/01/2026 19:21'
fn format_recycle_date(raw: &str) -> String {
    // Input format: '2026/01/12:19:21:00.000'
    // Split by : to separate date from time parts
    let parts: Vec<&str> = raw.split(':').collect();
    if parts.len() >= 3 {
        // parts[0] = "2026/01/12"
        // parts[1] = "19" (hour)
        // parts[2] = "21" (minute)
        let date_parts: Vec<&str> = parts[0].split('/').collect();
        if date_parts.len() == 3 {
            // date_parts = ["2026", "01", "12"]
            return format!(
                "{}/{}/{} {}:{}",
                date_parts[2], // day
                date_parts[1], // month
                date_parts[0], // year
                parts[1],      // hour
                parts[2]       // minute
            );
        }
    }
    // Fallback: return as-is but cleaner
    raw.replace(":", " ").trim_end_matches(".000").to_string()
}

/// Legacy function for backwards compatibility - enumerates all at once
pub fn enumerate_recycle_bin() -> Result<Vec<RecycleBinItem>> {
    unsafe {
        eprintln!("[Lixeira] Starting enumeration...");

        let mut items = Vec::new();
        let _com = ComApartmentGuard::init_sta_best_effort();

        let recycle_bin_folder: IShellItem =
            SHGetKnownFolderItem(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, None)?;

        let enum_items: IEnumShellItems =
            recycle_bin_folder.BindToHandler(None, &BHID_EnumItems)?;

        loop {
            let mut shell_items: [Option<IShellItem>; 1] = [None];
            let mut fetched: u32 = 0;

            if enum_items
                .Next(&mut shell_items, Some(&mut fetched))
                .is_err()
                || fetched == 0
            {
                break;
            }

            if let Some(shell_item) = shell_items[0].take() {
                if let Some(item) = process_shell_item(&shell_item) {
                    items.push(item);
                }
            }
        }

        eprintln!(
            "[Lixeira] Enumeration complete. Total items: {}",
            items.len()
        );
        Ok(items)
    }
}

/// Reads the deletion date from the corresponding $I metadata file in the Recycle Bin.
/// The $I file contains the Int64 FILETIME at offset 16.
/// Extracts the "Date Deleted" from the Recycle Bin's detailed view (Column Index 2)
unsafe fn get_date_deleted_from_pidl(folder: &IShellFolder, pidl: *const ITEMIDLIST) -> String {
    // Tenta converter (cast) a pasta para IShellFolder2
    let folder2: IShellFolder2 = match folder.cast() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[Lixeira] Failed to cast to IShellFolder2: {:?}", e);
            return "N/A".to_string();
        }
    };

    let mut sd = SHELLDETAILS::default();

    // O índice 2 na Lixeira (Recycle Bin) é padrão para "Data de Exclusão" no Windows
    // GetDetailsOf espera *const ITEMIDLIST, não Option
    match folder2.GetDetailsOf(pidl, 2, &mut sd) {
        Ok(_) => {
            // Converter o formato STRRET (que é arcaico) para string Rust
            let mut buffer = [0u16; 260]; // MAX_PATH
            if windows::Win32::UI::Shell::StrRetToBufW(
                std::ptr::addr_of_mut!(sd.str),
                Some(pidl),
                &mut buffer,
            )
            .is_ok()
            {
                let date_str = PCWSTR::from_raw(buffer.as_ptr())
                    .to_string()
                    .unwrap_or_default();
                if !date_str.is_empty() {
                    // Remove caracteres de controle invisíveis LTR/RTL que o Windows às vezes insere
                    let cleaned = date_str
                        .chars()
                        .filter(|c: &char| !c.is_control())
                        .collect::<String>()
                        .trim()
                        .to_string();
                    eprintln!("[Lixeira] Date from column 2: '{}'", cleaned);
                    return cleaned;
                }
            }
        }
        Err(e) => {
            eprintln!("[Lixeira] GetDetailsOf failed: {:?}", e);
        }
    }

    // Try alternate column indices - sometimes column order differs
    for col_idx in [3, 4, 5] {
        let mut sd2 = SHELLDETAILS::default();
        if folder2.GetDetailsOf(pidl, col_idx, &mut sd2).is_ok() {
            let mut buffer = [0u16; 260];
            if windows::Win32::UI::Shell::StrRetToBufW(
                std::ptr::addr_of_mut!(sd2.str),
                Some(pidl),
                &mut buffer,
            )
            .is_ok()
            {
                let col_str = PCWSTR::from_raw(buffer.as_ptr())
                    .to_string()
                    .unwrap_or_default();
                eprintln!("[Lixeira] Column {} value: '{}'", col_idx, col_str);
            }
        }
    }

    "Desconhecido".to_string()
}

/// Restore a file from the Recycle Bin to its original location
pub fn restore_from_recycle_bin(
    physical_path: &std::path::Path,
    original_path: &std::path::Path,
) -> Result<()> {
    unsafe {
        let _com = ComApartmentGuard::init_sta_best_effort();

        // Use IFileOperation for undo/restore
        let file_op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;

        // Set operation flags
        file_op.SetOperationFlags(FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT)?;

        // Create shell item for the physical path ($R file)
        let physical_str = physical_path.to_string_lossy();
        let physical_wide: Vec<u16> = physical_str.encode_utf16().chain(Some(0)).collect();
        let source_item: IShellItem =
            SHCreateItemFromParsingName(PCWSTR::from_raw(physical_wide.as_ptr()), None)?;

        // Create shell item for destination folder
        let dest_folder = original_path.parent().unwrap_or(original_path);
        let dest_str = dest_folder.to_string_lossy();
        let dest_wide: Vec<u16> = dest_str.encode_utf16().chain(Some(0)).collect();
        let dest_folder_item: IShellItem =
            SHCreateItemFromParsingName(PCWSTR::from_raw(dest_wide.as_ptr()), None)?;

        // Get original filename
        let file_name = original_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(Error::from_win32)?;
        let name_wide: Vec<u16> = file_name.encode_utf16().chain(Some(0)).collect();

        // Move the item
        file_op.MoveItem(
            &source_item,
            &dest_folder_item,
            PCWSTR::from_raw(name_wide.as_ptr()),
            None,
        )?;
        file_op.PerformOperations()?;

        Ok(())
    }
}

/// Permanently delete a file from the Recycle Bin.
/// Shows native Windows confirmation dialog before deleting.
pub fn delete_permanently(physical_path: &std::path::Path, hwnd: HWND) -> Result<()> {
    unsafe {
        let _com = ComApartmentGuard::init_sta_best_effort();

        // Use IFileOperation for permanent deletion
        let file_op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;

        // Set owner window for the confirmation dialog
        file_op.SetOwnerWindow(hwnd)?;

        // No FOF_ALLOWUNDO = permanent. No FOF_NOCONFIRMATION = Windows shows confirmation.
        file_op.SetOperationFlags(FOF_NOERRORUI)?;

        // Create shell item for the physical path
        let path_str = physical_path.to_string_lossy();
        let path_wide: Vec<u16> = path_str.encode_utf16().chain(Some(0)).collect();
        let item: IShellItem =
            SHCreateItemFromParsingName(PCWSTR::from_raw(path_wide.as_ptr()), None)?;

        // Delete the item permanently
        file_op.DeleteItem(&item, None)?;
        file_op.PerformOperations()?;

        // Check if user cancelled
        if file_op.GetAnyOperationsAborted()?.as_bool() {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_ABORT,
            ));
        }

        Ok(())
    }
}

/// Empty the entire Recycle Bin.
/// Shows native Windows confirmation dialog before emptying.
pub fn empty_recycle_bin(hwnd: HWND) -> Result<()> {
    unsafe {
        // SHEmptyRecycleBinW with NULL path empties all drives.
        // No SHERB_NOCONFIRMATION = Windows shows native confirmation dialog.
        SHEmptyRecycleBinW(Some(hwnd), PCWSTR::default(), SHERB_NOPROGRESSUI)?;
        Ok(())
    }
}
