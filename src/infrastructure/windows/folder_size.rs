//! Fast parallel folder size calculation using Win32 APIs
//!
//! Optimizations:
//! - FindFirstFileExW with FindExInfoBasic + FIND_FIRST_EX_LARGE_FETCH (batch I/O)
//! - Wide-string (Vec<u16>) path building — zero UTF-16↔UTF-8 round-trips
//! - Thread-local size accumulation — one atomic add per directory (not per file)
//! - Dedicated rayon thread pool with extra threads for I/O concurrency
//! - Breadth-first directory collection for better parallel work distribution

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW,
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FIND_FIRST_EX_LARGE_FETCH,
    WIN32_FIND_DATAW,
};

/// Cached rayon thread pool for folder-size calculations.
/// Previous code created a new pool on every call to `calculate_folder_size_parallel()`,
/// spawning `num_cpus * 2` OS threads each time. Pools may not be torn down immediately,
/// leading to thread/kernel-handle accumulation over prolonged use.
static FOLDER_SIZE_POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();

fn get_folder_size_pool() -> &'static rayon::ThreadPool {
    FOLDER_SIZE_POOL.get_or_init(|| {
        // num_cpus is sufficient for I/O-bound directory enumeration.
        // Previous `num_cpus * 2` was overkill and wasted ~16 OS threads on 16-core machines.
        let num_threads = num_cpus().max(4);
        rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .thread_name(|i| format!("folder-size-{}", i))
            .build()
            .expect("Failed to create folder-size rayon thread pool")
    })
}

/// Reparse tags for junctions/symlinks — only these redirect to other locations.
const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA0000003;
const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000000C;

/// Calculate a folder's total size using parallel Win32 directory enumeration.
///
/// Returns `None` if cancelled, `Some(total_bytes)` otherwise.
pub fn calculate_folder_size_parallel(
    root: &Path,
    cancel: &Arc<AtomicBool>,
    progress_callback: impl Fn(u64) + Send + Sync,
) -> Option<u64> {
    let total = Arc::new(AtomicU64::new(0));
    let progress_cb = Arc::new(progress_callback);

    // Convert root to wide string once
    let root_wide = path_to_wide(root);

    if cancel.load(Ordering::Relaxed) {
        return None;
    }

    // Phase 1: Breadth-first collection of first 2 levels to build a good work queue
    let mut level1_dirs: Vec<Vec<u16>> = Vec::new();
    scan_dir_wide(&root_wide, &total, &mut level1_dirs);
    (progress_cb)(total.load(Ordering::Relaxed));

    if cancel.load(Ordering::Relaxed) {
        return None;
    }

    // Expand one more level to get more parallel work units
    let mut work_queue: Vec<Vec<u16>> = Vec::with_capacity(level1_dirs.len() * 4);
    for dir in &level1_dirs {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let mut sub_dirs: Vec<Vec<u16>> = Vec::new();
        scan_dir_wide(dir, &total, &mut sub_dirs);
        if sub_dirs.is_empty() {
            // Leaf directory — already counted
        } else {
            work_queue.extend(sub_dirs);
        }
    }
    (progress_cb)(total.load(Ordering::Relaxed));

    if cancel.load(Ordering::Relaxed) {
        return None;
    }

    // Phase 2: Parallel recursive scan with dedicated I/O thread pool (reused across calls)
    let pool = get_folder_size_pool();

    pool.install(|| {
        parallel_scan_recursive(work_queue, &total, cancel, &progress_cb);
    });

    if cancel.load(Ordering::Relaxed) {
        return None;
    }

    Some(total.load(Ordering::Relaxed))
}

/// Recursively scan directories in parallel using rayon.
/// - Single-child chains (node_modules/a/b/c/...) → tight inline loop (zero rayon overhead)
/// - Multi-child branches → yield to rayon work-stealing pool for parallelism
fn parallel_scan_recursive(
    dirs: Vec<Vec<u16>>,
    total: &Arc<AtomicU64>,
    cancel: &Arc<AtomicBool>,
    progress_cb: &Arc<impl Fn(u64) + Send + Sync>,
) {
    use rayon::prelude::*;

    dirs.into_par_iter().for_each(|dir| {
        if cancel.load(Ordering::Relaxed) {
            return;
        }

        // Process this dir and follow single-child chains inline
        let mut current = dir;
        let mut local_total: u64 = 0;
        let mut dirs_inline: u32 = 0;

        loop {
            if cancel.load(Ordering::Relaxed) {
                break;
            }

            let mut sub_dirs: Vec<Vec<u16>> = Vec::new();
            local_total += scan_dir_wide_local(&current, &mut sub_dirs);
            dirs_inline += 1;

            // Flush accumulated size periodically
            if dirs_inline.is_multiple_of(64) && local_total > 0 {
                total.fetch_add(local_total, Ordering::Relaxed);
                local_total = 0;
                (progress_cb)(total.load(Ordering::Relaxed));
            }

            match sub_dirs.len() {
                0 => break, // Leaf — done with this chain
                1 => {
                    // Single child → continue inline (no rayon overhead)
                    if let Some(next_dir) = sub_dirs.pop() {
                        current = next_dir;
                    } else {
                        break;
                    }
                }
                _ => {
                    // Multiple children → flush and yield to rayon for parallelism
                    if local_total > 0 {
                        total.fetch_add(local_total, Ordering::Relaxed);
                        local_total = 0;
                    }
                    (progress_cb)(total.load(Ordering::Relaxed));
                    parallel_scan_recursive(sub_dirs, total, cancel, progress_cb);
                    break;
                }
            }
        }

        // Flush remaining
        if local_total > 0 {
            total.fetch_add(local_total, Ordering::Relaxed);
        }
    });
}

/// Scan a single directory. Returns the total file size found.
/// Pushes subdirectory wide-paths onto `sub_dirs`.
/// Uses wide-string paths throughout — zero UTF conversions.
fn scan_dir_wide_local(dir_wide: &[u16], sub_dirs: &mut Vec<Vec<u16>>) -> u64 {
    // Build search pattern: dir\* (wide string, null-terminated)
    let search = build_search_pattern(dir_wide);
    let mut find_data = WIN32_FIND_DATAW::default();
    let mut local_size: u64 = 0;

    unsafe {
        let handle = FindFirstFileExW(
            PCWSTR(search.as_ptr()),
            FindExInfoBasic,
            &mut find_data as *mut _ as *mut std::ffi::c_void,
            FindExSearchNameMatch,
            Some(std::ptr::null_mut()),
            FIND_FIRST_EX_LARGE_FETCH,
        );

        let handle = match handle {
            Ok(h) => h,
            Err(_) => return 0,
        };

        loop {
            let attrs = find_data.dwFileAttributes;
            let is_dir = (attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0;

            if is_dir {
                if !is_dot_or_dotdot(&find_data.cFileName)
                    && !is_junction_or_symlink(attrs, find_data.dwReserved0)
                {
                    sub_dirs.push(build_child_wide(dir_wide, &find_data.cFileName));
                }
            } else {
                local_size +=
                    ((find_data.nFileSizeHigh as u64) << 32) | (find_data.nFileSizeLow as u64);
            }

            if FindNextFileW(handle, &mut find_data).is_err() {
                break;
            }
        }

        let _ = FindClose(handle);
    }

    local_size
}

/// Scan a single directory — variant that uses Arc<AtomicU64> (for top-level scans).
fn scan_dir_wide(dir_wide: &[u16], total: &Arc<AtomicU64>, sub_dirs: &mut Vec<Vec<u16>>) {
    let size = scan_dir_wide_local(dir_wide, sub_dirs);
    if size > 0 {
        total.fetch_add(size, Ordering::Relaxed);
    }
}

// ── Helper functions ────────────────────────────────────────────────────────

/// Convert a `Path` to a null-terminated wide string (no trailing backslash).
fn path_to_wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    // Remove trailing backslash if present (but keep root like C:\)
    if wide.len() > 3 && wide.last() == Some(&(b'\\' as u16)) {
        wide.pop();
    }
    wide
}

/// Build a search pattern `dir\*\0` from a wide-string directory path.
#[inline]
fn build_search_pattern(dir_wide: &[u16]) -> Vec<u16> {
    let mut pattern = Vec::with_capacity(dir_wide.len() + 3);
    pattern.extend_from_slice(dir_wide);
    if pattern.last() != Some(&(b'\\' as u16)) {
        pattern.push(b'\\' as u16);
    }
    pattern.push(b'*' as u16);
    pattern.push(0); // null terminator
    pattern
}

/// Build a child path `parent\name` as wide string from parent wide + cFileName.
#[inline]
fn build_child_wide(parent_wide: &[u16], c_file_name: &[u16]) -> Vec<u16> {
    let name_len = c_file_name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(c_file_name.len());

    let mut child = Vec::with_capacity(parent_wide.len() + 1 + name_len);
    child.extend_from_slice(parent_wide);
    if child.last() != Some(&(b'\\' as u16)) {
        child.push(b'\\' as u16);
    }
    child.extend_from_slice(&c_file_name[..name_len]);
    child
}

/// Check if cFileName is "." or ".."
#[inline(always)]
fn is_dot_or_dotdot(name: &[u16]) -> bool {
    let first = name[0];
    if first != b'.' as u16 {
        return false;
    }
    let second = name[1];
    second == 0 || (second == b'.' as u16 && name[2] == 0)
}

/// Check if a reparse point is a junction or symlink (should be skipped).
#[inline(always)]
fn is_junction_or_symlink(attrs: u32, reparse_tag: u32) -> bool {
    if (attrs & FILE_ATTRIBUTE_REPARSE_POINT.0) == 0 {
        return false;
    }
    reparse_tag == IO_REPARSE_TAG_MOUNT_POINT || reparse_tag == IO_REPARSE_TAG_SYMLINK
}

/// Get number of logical CPUs.
fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}
