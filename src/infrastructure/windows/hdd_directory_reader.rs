//! HDD-optimized directory listing using Win32 APIs
//!
//! This module provides optimized directory scanning for mechanical HDDs using:
//! - FindFirstFileExW with FIND_FIRST_EX_LARGE_FETCH for sequential reads
//! - FindExInfoBasic to skip 8.3 short name generation
//! - One-pass metadata extraction from WIN32_FIND_DATAW
//! - Batched results to minimize channel contention
//! - Thread priority management for HDD operations

use std::path::Path;
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW,
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM,
    FIND_FIRST_EX_LARGE_FETCH, WIN32_FIND_DATAW,
};
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_NORMAL,
};

use crate::domain::file_entry::FileEntry;
use crate::infrastructure::onedrive;

/// Batch size for sending results via channel (reduces lock contention)
const BATCH_SIZE: usize = 500;

/// HDD-optimized directory entry with metadata from WIN32_FIND_DATAW
#[derive(Debug, Clone)]
pub struct HddDirectoryEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64,
    pub attributes: u32,
}

/// Read directory contents optimized for HDD performance
///
/// Uses FindFirstFileExW with LARGE_FETCH flag to read directory table in chunks,
/// minimizing seek operations on mechanical drives.
pub fn read_directory_hdd_optimized(
    path: &Path,
    is_onedrive: bool,
    show_hidden: bool,
) -> Result<Vec<FileEntry>, String> {
    // Set thread priority to normal/above normal for HDD operations
    // Background priority causes HDD head to seek away for other tasks
    unsafe {
        let thread = GetCurrentThread();
        let _ = SetThreadPriority(thread, THREAD_PRIORITY_NORMAL);
    }

    let result = read_directory_impl(path, is_onedrive, show_hidden);

    // Reset thread priority after operation
    unsafe {
        let thread = GetCurrentThread();
        let _ = SetThreadPriority(thread, THREAD_PRIORITY_NORMAL);
    }

    result
}

/// Get batches of directory entries for HDD optimization
///
/// Builds batches during enumeration (true streaming) instead of
/// collecting all entries first and splitting afterwards.
/// Returns entries in chunks of BATCH_SIZE to minimize channel contention.
pub fn read_directory_hdd_batched(
    path: &Path,
    is_onedrive: bool,
    show_hidden: bool,
) -> Result<Vec<Vec<FileEntry>>, String> {
    // Set thread priority to normal for HDD operations
    unsafe {
        let thread = GetCurrentThread();
        let _ = SetThreadPriority(thread, THREAD_PRIORITY_NORMAL);
    }

    let result = read_directory_impl_batched(path, is_onedrive, show_hidden);

    // Reset thread priority after operation
    unsafe {
        let thread = GetCurrentThread();
        let _ = SetThreadPriority(thread, THREAD_PRIORITY_NORMAL);
    }

    result
}

/// Internal batched implementation — builds Vec<Vec<FileEntry>> during
/// the FindFirstFileExW loop, avoiding the double-pass of collect-then-split.
fn read_directory_impl_batched(
    path: &Path,
    is_onedrive: bool,
    show_hidden: bool,
) -> Result<Vec<Vec<FileEntry>>, String> {
    let search_path = if path.to_string_lossy().ends_with('\\') {
        format!("{}*", path.display())
    } else {
        format!("{}\\*", path.display())
    };

    let wide_path: Vec<u16> = search_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let mut find_data = WIN32_FIND_DATAW::default();
    let mut batches = Vec::new();
    let mut current_batch = Vec::with_capacity(BATCH_SIZE);

    unsafe {
        let handle = FindFirstFileExW(
            PCWSTR(wide_path.as_ptr()),
            FindExInfoBasic,
            &mut find_data as *mut _ as *mut std::ffi::c_void,
            FindExSearchNameMatch,
            Some(std::ptr::null_mut()),
            FIND_FIRST_EX_LARGE_FETCH,
        );

        let handle = match handle {
            Ok(handle) => handle,
            Err(_) => {
                return Err(format!("Failed to open directory: {}", path.display()));
            }
        };

        loop {
            let filename = extract_filename(&find_data.cFileName)?;

            if filename == "." || filename == ".." {
                if FindNextFileW(handle, &mut find_data).is_err() {
                    break;
                }
                continue;
            }

            let attrs = find_data.dwFileAttributes;
            let is_hidden = (attrs & FILE_ATTRIBUTE_HIDDEN.0) != 0;
            let is_system = (attrs & FILE_ATTRIBUTE_SYSTEM.0) != 0;
            if is_system || (!show_hidden && is_hidden) {
                if FindNextFileW(handle, &mut find_data).is_err() {
                    break;
                }
                continue;
            }

            let entry = create_file_entry(&find_data, path, &filename, is_onedrive)?;

            if should_include_entry(&entry) {
                current_batch.push(entry);
                if current_batch.len() >= BATCH_SIZE {
                    batches.push(current_batch);
                    current_batch = Vec::with_capacity(BATCH_SIZE);
                }
            }

            if FindNextFileW(handle, &mut find_data).is_err() {
                break;
            }
        }

        let _ = FindClose(handle);
    }

    if !current_batch.is_empty() {
        batches.push(current_batch);
    }

    Ok(batches)
}

/// Internal implementation using Win32 APIs
fn read_directory_impl(
    path: &Path,
    is_onedrive: bool,
    show_hidden: bool,
) -> Result<Vec<FileEntry>, String> {
    let search_path = if path.to_string_lossy().ends_with('\\') {
        format!("{}*", path.display())
    } else {
        format!("{}\\*", path.display())
    };

    // Convert to wide string
    let wide_path: Vec<u16> = search_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let mut find_data = WIN32_FIND_DATAW::default();
    let mut entries = Vec::new();

    unsafe {
        // Use FindFirstFileExW with optimization flags
        let handle = FindFirstFileExW(
            PCWSTR(wide_path.as_ptr()),
            FindExInfoBasic, // Skip 8.3 short name generation
            &mut find_data as *mut _ as *mut std::ffi::c_void,
            FindExSearchNameMatch,      // Standard name matching
            Some(std::ptr::null_mut()), // No additional search criteria
            FIND_FIRST_EX_LARGE_FETCH,  // Critical: Read larger directory chunks
        );

        let handle = match handle {
            Ok(handle) => handle,
            Err(_) => {
                return Err(format!("Failed to open directory: {}", path.display()));
            }
        };

        loop {
            // Extract filename from wide string
            let filename = extract_filename(&find_data.cFileName)?;

            // Skip special entries
            if filename == "." || filename == ".." {
                if FindNextFileW(handle, &mut find_data).is_err() {
                    break;
                }
                continue;
            }

            // Check hidden attribute before creating entry
            let attrs = find_data.dwFileAttributes;
            let is_hidden = (attrs & FILE_ATTRIBUTE_HIDDEN.0) != 0;
            let is_system = (attrs & FILE_ATTRIBUTE_SYSTEM.0) != 0;
            if is_system || (!show_hidden && is_hidden) {
                if FindNextFileW(handle, &mut find_data).is_err() {
                    break;
                }
                continue;
            }

            // Extract metadata in one pass
            let entry = create_file_entry(&find_data, path, &filename, is_onedrive)?;

            // Apply filters
            if should_include_entry(&entry) {
                entries.push(entry);
            }

            // Get next entry
            if FindNextFileW(handle, &mut find_data).is_err() {
                break;
            }
        }

        // Always close the handle
        let _ = FindClose(handle);
    }

    Ok(entries)
}

/// Extract filename from wide character array
fn extract_filename(wide_name: &[u16]) -> Result<String, String> {
    let len = wide_name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(wide_name.len());

    if len == 0 {
        return Err("Empty filename".to_string());
    }

    Ok(String::from_utf16_lossy(&wide_name[0..len]))
}

/// Create FileEntry from WIN32_FIND_DATAW in one pass
fn create_file_entry(
    find_data: &WIN32_FIND_DATAW,
    base_path: &Path,
    filename: &str,
    is_onedrive: bool,
) -> Result<FileEntry, String> {
    let full_path = base_path.join(filename);

    // Extract attributes
    let attributes = find_data.dwFileAttributes;
    let is_hidden = (attributes & FILE_ATTRIBUTE_HIDDEN.0) != 0;
    let _is_system = (attributes & FILE_ATTRIBUTE_SYSTEM.0) != 0;
    let is_directory = (attributes & FILE_ATTRIBUTE_DIRECTORY.0) != 0;

    // Handle archive files as directories
    let mut is_dir = is_directory;
    let is_archive = crate::domain::file_entry::is_archive_extension(filename);
    if !is_dir && is_archive {
        is_dir = true;
    }

    // Extract file size (combine high and low 32-bit values)
    let size = if is_dir && !is_archive {
        0
    } else {
        ((find_data.nFileSizeHigh as u64) << 32) | (find_data.nFileSizeLow as u64)
    };

    // Extract modification time (Windows FILETIME to Unix timestamp)
    let ft = find_data.ftLastWriteTime;
    let windows_ticks = ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
    let modified = if windows_ticks > 116444736000000000 {
        (windows_ticks - 116444736000000000) / 10_000_000
    } else {
        0
    };

    // Get sync status from attributes (OneDrive flags are already included)
    let sync_status = onedrive::get_sync_status(attributes, is_onedrive);

    Ok(FileEntry {
        path: full_path,
        name: filename.to_string(),
        is_dir,
        size,
        modified,
        folder_cover: None, // Will be populated later if needed
        drive_info: None,
        sync_status,
        is_hidden,
        recycle_bin: None,
    })
}

/// Determine if entry should be included in results
fn should_include_entry(entry: &FileEntry) -> bool {
    // Skip hidden/system files
    if entry.name.starts_with('.') {
        return false;
    }

    // Skip special system files
    !matches!(
        entry.name.to_lowercase().as_str(),
        "desktop.ini" | "thumbs.db" | "$recycle.bin" | "system volume information"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::file_entry::SyncStatus;
    use std::path::PathBuf;

    #[test]
    fn test_should_include_entry() {
        let test_entry = |name: &str| -> FileEntry {
            FileEntry {
                path: PathBuf::from(format!("C:\\test\\{}", name)),
                name: name.to_string(),
                is_dir: false,
                size: 100,
                modified: 1234567890,
                folder_cover: None,
                drive_info: None,
                sync_status: SyncStatus::None,
                is_hidden: false,
                recycle_bin: None,
            }
        };

        assert!(!should_include_entry(&test_entry(".hidden")));
        assert!(!should_include_entry(&test_entry("desktop.ini")));
        assert!(!should_include_entry(&test_entry("Thumbs.db")));
        assert!(!should_include_entry(&test_entry("$Recycle.Bin")));
        assert!(should_include_entry(&test_entry("normal_file.txt")));
        assert!(should_include_entry(&test_entry("document.pdf")));
    }
}
