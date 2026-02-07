//! Fast parallel folder size calculation using Win32 APIs
//!
//! Uses FindFirstFileExW with FindExInfoBasic + FIND_FIRST_EX_LARGE_FETCH
//! to get file sizes directly from directory enumeration (zero extra syscalls).
//! Subdirectories are traversed in parallel using rayon for maximum throughput.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW,
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FIND_FIRST_EX_LARGE_FETCH,
    WIN32_FIND_DATAW,
};

/// Calculate a folder's total size using parallel Win32 directory enumeration.
///
/// - `cancel`: set to `true` to abort early
/// - `progress_callback`: called periodically with the running total; return from it quickly.
///
/// Returns `None` if cancelled, `Some(total_bytes)` otherwise.
pub fn calculate_folder_size_parallel(
    root: &Path,
    cancel: &Arc<AtomicBool>,
    progress_callback: impl Fn(u64) + Send + Sync,
) -> Option<u64> {
    let total = Arc::new(AtomicU64::new(0));
    let cancelled = Arc::new(AtomicBool::new(false));
    let progress_cb = Arc::new(progress_callback);

    // Gather immediate children: files contribute size, dirs go into work list
    let mut dir_stack: Vec<PathBuf> = Vec::new();

    if cancel.load(Ordering::Relaxed) {
        return None;
    }

    // Scan root directory (single-threaded) to collect first-level subdirs
    scan_directory_sizes(root, &total, &mut dir_stack);

    // Early progress report after root scan
    (progress_cb)(total.load(Ordering::Relaxed));

    if cancel.load(Ordering::Relaxed) {
        return None;
    }

    // Process all subdirectories in parallel using rayon
    // We use a recursive parallel approach: for each directory, scan it,
    // collect its subdirs, and recurse in parallel.
    let cancel_ref = cancel.clone();
    let total_ref = total.clone();
    let cancelled_ref = cancelled.clone();
    let progress_ref = progress_cb.clone();

    // Use a counter to emit progress periodically
    let dirs_processed = Arc::new(AtomicU64::new(0));

    parallel_scan_dirs(
        dir_stack,
        &total_ref,
        &cancel_ref,
        &cancelled_ref,
        &progress_ref,
        &dirs_processed,
    );

    if cancelled.load(Ordering::Relaxed) || cancel.load(Ordering::Relaxed) {
        return None;
    }

    Some(total.load(Ordering::Relaxed))
}

/// Recursively scan directories in parallel using rayon.
fn parallel_scan_dirs(
    dirs: Vec<PathBuf>,
    total: &Arc<AtomicU64>,
    cancel: &Arc<AtomicBool>,
    cancelled: &Arc<AtomicBool>,
    progress_cb: &Arc<impl Fn(u64) + Send + Sync>,
    dirs_processed: &Arc<AtomicU64>,
) {
    use rayon::prelude::*;

    dirs.into_par_iter().for_each(|dir| {
        if cancel.load(Ordering::Relaxed) {
            cancelled.store(true, Ordering::Relaxed);
            return;
        }

        let mut sub_dirs: Vec<PathBuf> = Vec::new();
        scan_directory_sizes(&dir, total, &mut sub_dirs);

        // Emit progress every 32 directories
        let count = dirs_processed.fetch_add(1, Ordering::Relaxed);
        if count % 32 == 0 {
            (progress_cb)(total.load(Ordering::Relaxed));
        }

        // Recurse into subdirectories (rayon handles work-stealing)
        if !sub_dirs.is_empty() {
            parallel_scan_dirs(sub_dirs, total, cancel, cancelled, progress_cb, dirs_processed);
        }
    });
}

/// Scan a single directory using FindFirstFileExW.
/// Adds file sizes to `total` and pushes subdirectory paths to `sub_dirs`.
fn scan_directory_sizes(
    dir: &Path,
    total: &Arc<AtomicU64>,
    sub_dirs: &mut Vec<PathBuf>,
) {
    let search_path = if dir.to_string_lossy().ends_with('\\') {
        format!("{}*", dir.display())
    } else {
        format!("{}\\*", dir.display())
    };

    let wide_path: Vec<u16> = search_path.encode_utf16().chain(std::iter::once(0)).collect();

    let mut find_data = WIN32_FIND_DATAW::default();

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
            Ok(h) => h,
            Err(_) => return, // Access denied or invalid path — skip silently
        };

        loop {
            process_find_entry(&find_data, dir, total, sub_dirs);

            if FindNextFileW(handle, &mut find_data).is_err() {
                break;
            }
        }

        let _ = FindClose(handle);
    }
}

/// Reparse tags that indicate the directory is a redirect to another location.
/// Only these should be skipped; other reparse types (OneDrive, WOF, etc.) are real dirs.
const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA0000003; // Junction points
const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000000C; // Symbolic links

/// Process a single WIN32_FIND_DATAW entry.
#[inline(always)]
fn process_find_entry(
    find_data: &WIN32_FIND_DATAW,
    parent_dir: &Path,
    total: &Arc<AtomicU64>,
    sub_dirs: &mut Vec<PathBuf>,
) {
    let attrs = find_data.dwFileAttributes;
    let is_dir = (attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0;

    if is_dir {
        // Skip "." and ".."
        let first = find_data.cFileName[0];
        if first == b'.' as u16 {
            let second = find_data.cFileName[1];
            if second == 0 || (second == b'.' as u16 && find_data.cFileName[2] == 0) {
                return;
            }
        }

        // Skip only junction points and symlinks to avoid double-counting
        // and infinite loops (e.g. C:\Users\All Users → C:\ProgramData).
        // Other reparse types (OneDrive cloud dirs, WOF, etc.) are real content.
        let is_reparse = (attrs & FILE_ATTRIBUTE_REPARSE_POINT.0) != 0;
        if is_reparse {
            let tag = find_data.dwReserved0;
            if tag == IO_REPARSE_TAG_MOUNT_POINT || tag == IO_REPARSE_TAG_SYMLINK {
                return;
            }
        }

        // Build full path for subdirectory
        let name_len = find_data
            .cFileName
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(find_data.cFileName.len());
        let name = String::from_utf16_lossy(&find_data.cFileName[..name_len]);
        sub_dirs.push(parent_dir.join(name));
    } else {
        // File: extract size directly from WIN32_FIND_DATAW (no extra syscall)
        let size = ((find_data.nFileSizeHigh as u64) << 32) | (find_data.nFileSizeLow as u64);
        total.fetch_add(size, Ordering::Relaxed);
    }
}
