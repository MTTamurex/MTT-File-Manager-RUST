use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use windows::core::Interface;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::Common::*;
use windows::Win32::UI::Shell::*;

use super::{
    ComApartmentGuard, RecycleBinItem, PKEY_ITEMNAMEDISPLAY, PKEY_RECYCLE_DATE_DELETED,
    PKEY_RECYCLE_DELETED_FROM, PKEY_SIZE,
};

/// Own all PIDLs returned by one Next call, including unprocessed items when
/// navigation cancels the load or the batch callback unwinds.
struct PidlBatch([*mut ITEMIDLIST; 32]);

impl Drop for PidlBatch {
    fn drop(&mut self) {
        for pidl in self.0 {
            if !pidl.is_null() {
                unsafe { CoTaskMemFree(Some(pidl.cast())) };
            }
        }
    }
}

pub(super) fn enumerate_recycle_bin_streaming(
    mut on_batch: impl FnMut(Vec<RecycleBinItem>),
    generation: Arc<AtomicUsize>,
    my_gen: usize,
    batch_size: usize,
) -> Result<()> {
    if generation.load(Ordering::Relaxed) != my_gen {
        return Ok(());
    }
    unsafe {
        log::debug!("[Lixeira] Starting streaming enumeration (Shell API)...");

        let _com = ComApartmentGuard::init_sta_best_effort();

        // 1. Get Desktop Folder
        let desktop: IShellFolder = SHGetDesktopFolder()?;

        // 2. Get Recycle Bin PIDL
        let recycle_bin_pidl = SHGetKnownFolderIDList(&FOLDERID_RecycleBinFolder, 0, None)?;

        // 3. Bind to Recycle Bin Folder (IShellFolder)
        let recycle_bin_folder: IShellFolder = match desktop.BindToObject(recycle_bin_pidl, None) {
            Ok(folder) => folder,
            Err(error) => {
                CoTaskMemFree(Some(recycle_bin_pidl as *mut _));
                return Err(error);
            }
        };
        CoTaskMemFree(Some(recycle_bin_pidl as *mut _)); // Release PIDL

        // 4. Enumerate Objects (as PIDLs)
        // Fix SHCONTF flags: cast to u32 because EnumObjects takes u32
        let flags = (SHCONTF_FOLDERS.0 | SHCONTF_NONFOLDERS.0 | SHCONTF_INCLUDEHIDDEN.0) as u32;
        let mut enum_list_opt: Option<IEnumIDList> = None;

        let enum_result =
            recycle_bin_folder.EnumObjects(HWND::default(), flags, &mut enum_list_opt);
        if enum_result.is_err() {
            return Err(Error::from_hresult(enum_result));
        }
        let enum_list = match enum_list_opt {
            Some(list) => list,
            None => return Ok(()), // Empty Recycle Bin
        };

        let batch_size = batch_size.max(1);
        let mut batch = Vec::with_capacity(batch_size);
        let mut total_count = 0;

        loop {
            // Check cancellation
            if generation.load(Ordering::Relaxed) != my_gen {
                return Ok(());
            }

            let mut fetched: u32 = 0;
            let mut pidls = PidlBatch([std::ptr::null_mut(); 32]);

            // Fetch several PIDLs per COM call to reduce per-item enumeration overhead.
            let next_result = enum_list.Next(&mut pidls.0, Some(&mut fetched));
            if fetched == 0 {
                if next_result.is_err() {
                    return Err(Error::from_hresult(next_result));
                }
                break;
            }

            for pidl_child in pidls.0.iter().take(fetched as usize).copied() {
                if generation.load(Ordering::Relaxed) != my_gen {
                    return Ok(());
                }
                // SHCreateShellItem uses the current folder as the parent and this child PIDL.
                if let Ok(shell_item) =
                    SHCreateShellItem(None, Some(&recycle_bin_folder), pidl_child)
                {
                    if let Some(mut item) = process_shell_item(&shell_item) {
                        // GetDetailsOf is an extra Shell call; use it only when the item
                        // properties did not provide a usable deletion date.
                        if item.date_deleted_unix == 0
                            && crate::domain::date_format::DateFormat::YearMonthDay
                                .reformat_year_first(&item.date_deleted)
                                .is_none()
                        {
                            item.date_deleted =
                                get_date_deleted_from_pidl(&recycle_bin_folder, pidl_child);
                        }

                        batch.push(item);
                        total_count += 1;
                    }
                }

                if batch.len() >= batch_size {
                    if generation.load(Ordering::Relaxed) != my_gen {
                        return Ok(());
                    }
                    on_batch(std::mem::take(&mut batch));
                    batch = Vec::with_capacity(batch_size);
                }
            }

            if next_result.is_err() {
                return Err(Error::from_hresult(next_result));
            }
        }

        if !batch.is_empty() && generation.load(Ordering::Relaxed) == my_gen {
            on_batch(batch);
        }

        log::debug!(
            "[Lixeira] Enumeration complete (Shell API). Total items: {}",
            total_count
        );
        Ok(())
    }
}

/// Process a single shell item into RecycleBinItem.
unsafe fn process_shell_item(shell_item: &IShellItem) -> Option<RecycleBinItem> {
    let shell_item2: IShellItem2 = shell_item.cast().ok()?;

    // Get display name - this should be the original filename.
    let name = get_item_display_name(&shell_item2);

    // Get parent folder (where item was deleted from).
    let parent_folder = get_shell_item_string_property(&shell_item2, &PKEY_RECYCLE_DELETED_FROM)
        .unwrap_or_default();

    // Build full original path.
    let original_path = if !parent_folder.is_empty() {
        PathBuf::from(&parent_folder).join(&name)
    } else {
        PathBuf::from(&name)
    };

    // Get physical path (Parsing path) - this gives us the $R path in $Recycle.Bin.
    let physical_path =
        if let Ok(name_ptr) = shell_item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING) {
            let path_str = name_ptr.to_string().unwrap_or_default();
            CoTaskMemFree(Some(name_ptr.as_ptr() as *mut _));
            PathBuf::from(path_str)
        } else {
            PathBuf::new()
        };

    // Get extension for icon lookup.
    let extension = std::path::Path::new(&name)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        .unwrap_or_default();

    // Get date deleted as both UNIX timestamp and formatted string.
    let (date_deleted_unix, date_deleted) =
        get_shell_item_filetime_property(&shell_item2, &PKEY_RECYCLE_DATE_DELETED)
            .unwrap_or_else(|_| (0, rust_i18n::t!("file_info.unknown_date").to_string()));

    // Get size.
    let size = get_shell_item_u64_property(&shell_item2, &PKEY_SIZE).unwrap_or(0);

    // Check if directory.
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

/// Get the display name of an item (original filename).
unsafe fn get_item_display_name(item: &IShellItem2) -> String {
    // Try PKEY_ItemNameDisplay first - this gives the original filename.
    if let Ok(name) = get_shell_item_string_property(item, &PKEY_ITEMNAMEDISPLAY) {
        if !name.is_empty() && !name.starts_with("$R") && !name.contains("\\$Recycle") {
            return name;
        }
    }

    // Try SIGDN_NORMALDISPLAY.
    if let Ok(name_ptr) = item.GetDisplayName(SIGDN_NORMALDISPLAY) {
        let name = name_ptr.to_string().unwrap_or_default();
        CoTaskMemFree(Some(name_ptr.as_ptr() as *mut _));
        if !name.is_empty() && !name.starts_with("$R") && !name.contains("\\$Recycle") {
            return name;
        }
    }

    // Try SIGDN_PARENTRELATIVEEDITING - sometimes has better name.
    if let Ok(name_ptr) = item.GetDisplayName(SIGDN_PARENTRELATIVEEDITING) {
        let name = name_ptr.to_string().unwrap_or_default();
        CoTaskMemFree(Some(name_ptr.as_ptr() as *mut _));
        if !name.is_empty() && !name.starts_with("$R") {
            return name;
        }
    }

    // Try SIGDN_PARENTRELATIVEFORADDRESSBAR as last resort.
    if let Ok(name_ptr) = item.GetDisplayName(SIGDN_PARENTRELATIVEFORADDRESSBAR) {
        let name = name_ptr.to_string().unwrap_or_default();
        CoTaskMemFree(Some(name_ptr.as_ptr() as *mut _));
        // Extract just filename from any path-like result.
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

/// Get string property from IShellItem2.
unsafe fn get_shell_item_string_property(item: &IShellItem2, pkey: &PROPERTYKEY) -> Result<String> {
    let str_ptr = item.GetString(pkey)?;
    let result = str_ptr.to_string().map_err(|_| Error::from_thread())?;
    CoTaskMemFree(Some(str_ptr.0 as *mut _));
    Ok(result)
}

/// Get u64 property from IShellItem2.
unsafe fn get_shell_item_u64_property(item: &IShellItem2, pkey: &PROPERTYKEY) -> Result<u64> {
    item.GetUInt64(pkey)
}

/// Get FILETIME property from IShellItem2 and format as date string.
unsafe fn get_shell_item_filetime_property(
    item: &IShellItem2,
    pkey: &PROPERTYKEY,
) -> Result<(u64, String)> {
    if let Ok(str_ptr) = item.GetString(pkey) {
        let result = str_ptr.to_string().unwrap_or_default();
        CoTaskMemFree(Some(str_ptr.0 as *mut _));
        if !result.is_empty() {
            // Store a canonical source value, independent of the active UI format.
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

    // Fallback: Try PropVariantToUInt64 for raw FILETIME.
    if let Ok(ft) = item.GetFileTime(pkey) {
        // Fallback: raw FILETIME.
        let ft_val = ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
        if let Some(unix_secs) = filetime_to_unix_secs(ft_val) {
            return Ok((
                unix_secs,
                crate::infrastructure::windows::formatting::format_date_with_format(
                    unix_secs,
                    crate::domain::date_format::DateFormat::YearMonthDay,
                ),
            ));
        }
    }

    Err(Error::from_thread())
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

/// Keep source dates in an unambiguous, preference-independent format.
fn format_recycle_date(raw: &str) -> String {
    // Input format: '2026/01/12:19:21:00.000'
    // Split by : to separate date from time parts.
    let parts: Vec<&str> = raw.split(':').collect();
    if parts.len() >= 3 {
        // parts[0] = "2026/01/12"
        // parts[1] = "19" (hour)
        // parts[2] = "21" (minute)
        let date_parts: Vec<&str> = parts[0].split('/').collect();
        if date_parts.len() == 3 {
            // date_parts = ["2026", "01", "12"]
            return format!(
                "{}-{}-{} {}:{}",
                date_parts[0], // year
                date_parts[1], // month
                date_parts[2], // day
                parts[1],      // hour
                parts[2]       // minute
            );
        }
    }
    // Fallback: return as-is but cleaner.
    raw.replace(":", " ").trim_end_matches(".000").to_string()
}

/// Extracts the "Date Deleted" from the Recycle Bin detailed view (Column Index 2).
unsafe fn get_date_deleted_from_pidl(folder: &IShellFolder, pidl: *const ITEMIDLIST) -> String {
    // Try to cast the folder to IShellFolder2.
    let folder2: IShellFolder2 = match folder.cast() {
        Ok(f) => f,
        Err(e) => {
            log::error!("[Lixeira] Failed to cast to IShellFolder2: {:?}", e);
            return "N/A".to_string();
        }
    };

    let mut sd = SHELLDETAILS::default();

    // Index 2 in the Recycle Bin is the standard column for "Date Deleted" on Windows.
    // GetDetailsOf expects *const ITEMIDLIST, not Option.
    match folder2.GetDetailsOf(pidl, 2, &mut sd) {
        Ok(_) => {
            // Convert the archaic STRRET format to a Rust string.
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
                    // Remove invisible LTR/RTL control chars that Windows sometimes inserts.
                    let cleaned = date_str
                        .chars()
                        .filter(|c: &char| !c.is_control())
                        .collect::<String>()
                        .trim()
                        .to_string();
                    log::trace!("[Lixeira] Date from column 2: '{}'", cleaned);
                    return cleaned;
                }
            }
        }
        Err(e) => {
            log::warn!("[Lixeira] GetDetailsOf failed: {:?}", e);
        }
    }

    // Try alternate column indices - sometimes column order differs.
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
                log::trace!("[Lixeira] Column {} value: '{}'", col_idx, col_str);
            }
        }
    }

    rust_i18n::t!("file_info.unknown_date").to_string()
}

#[cfg(test)]
#[path = "date_tests.rs"]
mod date_tests;
