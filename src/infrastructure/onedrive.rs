//! OneDrive sync status detection utilities.
//!
//! This module provides functions to detect if a path is within a OneDrive
//! folder and to parse file attributes into sync status values.
//!
//! PERFORMANCE CRITICAL: All I/O operations on OneDrive files use timeout-based
//! wrappers to prevent indefinite blocking on cloud-only files.

use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::domain::file_entry::SyncStatus;

// Maximum number of concurrent timeout threads (prevents thread exhaustion)
const MAX_CONCURRENT_TIMEOUT_THREADS: u64 = 4;

// Counter of active timeout threads for monitoring and limiting
static ACTIVE_TIMEOUT_THREADS: AtomicU64 = AtomicU64::new(0);

// Global flag indicating if app is minimized (for operation cancellation)
static APP_MINIMIZED: AtomicBool = AtomicBool::new(false);

// Timeout configurations
const ONEDRIVE_METADATA_TIMEOUT_MINIMIZED_MS: u64 = 50;

// Cached OneDrive root paths (initialized once at startup)
static ONEDRIVE_ROOTS: OnceLock<Vec<String>> = OnceLock::new();

/// Set the minimized state of the application.
/// When minimized, timeout operations are cancelled more aggressively.
pub fn set_app_minimized(minimized: bool) {
    APP_MINIMIZED.store(minimized, Ordering::SeqCst);
    eprintln!("[ONEDRIVE LIFECYCLE] App minimized state changed: {}", minimized);
    if minimized {
        eprintln!("[ONEDRIVE LIFECYCLE] Active timeout threads at minimize: {}",
                  ACTIVE_TIMEOUT_THREADS.load(Ordering::SeqCst));
    }
}

/// Check if the application is currently minimized
pub fn is_app_minimized() -> bool {
    APP_MINIMIZED.load(Ordering::SeqCst)
}

/// Get the current count of active timeout threads (for monitoring)
pub fn get_active_timeout_threads() -> u64 {
    ACTIVE_TIMEOUT_THREADS.load(Ordering::SeqCst)
}

// NOTE: Cloud attribute detection is NOT cached per drive letter.
// Different paths on the same drive can have different cloud attributes
// (e.g., C:\Users\Docs = no cloud, C:\Users\OneDrive = cloud).
// GetFileAttributesW is a fast cached filesystem call, no disk I/O.

// Windows file attribute constants for cloud files (undocumented but well-known)
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x00400000;
const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x00040000; // File is being synced
const FILE_ATTRIBUTE_PINNED: u32 = 0x00080000;
const FILE_ATTRIBUTE_OFFLINE: u32 = 0x00001000;

/// Timeout for metadata operations on OneDrive files (milliseconds)
const ONEDRIVE_METADATA_TIMEOUT_MS: u64 = 100;
/// Timeout for exists check on OneDrive files (milliseconds)
const ONEDRIVE_EXISTS_TIMEOUT_MS: u64 = 50;
/// Timeout for directory enumeration on OneDrive (milliseconds) - CRITICAL
/// FindFirstFileW can block for 30-60s on OneDrive folders with cloud-only files
const ONEDRIVE_DIR_ENUM_TIMEOUT_MS: u64 = 5000;

/// Returns true if the attribute set contains any Cloud Files flags (OneDrive, iCloud, etc).
/// This acts as a fallback when the path-based detection fails (e.g., alternate mount points).
pub fn has_cloud_attributes(attrs: u32) -> bool {
    (attrs & FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0
        || (attrs & FILE_ATTRIBUTE_RECALL_ON_OPEN) != 0
        || (attrs & FILE_ATTRIBUTE_PINNED) != 0
        || (attrs & FILE_ATTRIBUTE_OFFLINE) != 0
}

/// Initialize OneDrive root paths from environment variables.
/// Should be called once at application startup.
pub fn init_onedrive_paths() {
    ONEDRIVE_ROOTS.get_or_init(|| {
        let mut roots = Vec::new();
        for var in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
            if let Ok(path) = std::env::var(var) {
                if !path.is_empty() {
                    roots.push(path.to_lowercase());
                }
            }
        }
        eprintln!("[OneDrive] Detected roots: {:?}", roots);
        roots
    });
}

/// Check if a path is within a OneDrive folder.
/// Uses cached roots from environment variables.
pub fn is_onedrive_path(path: &Path) -> bool {
    let path_lower = path.to_string_lossy().to_lowercase();
    ONEDRIVE_ROOTS
        .get()
        .map(|roots| roots.iter().any(|r| path_lower.starts_with(r)))
        .unwrap_or(false)
}

/// Fallback detection using file attributes for cases where the OneDrive root
/// isn't covered by environment variables (e.g., secondary business accounts).
///
/// Calls GetFileAttributesW directly (fast cached filesystem call, no disk I/O).
/// NOT cached per drive letter — different paths on the same drive can have
/// different cloud attributes (e.g., C:\Users\Docs vs C:\Users\OneDrive).
pub fn path_has_cloud_attributes(path: &Path) -> bool {
    check_cloud_attributes_uncached(path)
}

/// Fast file/directory existence check using GetFileAttributesW.
///
/// CRITICAL: Unlike `path.exists()` which uses `CreateFileW` + `GetFileInformationByHandle`
/// (can trigger OneDrive file recall/download, blocking for 30-60s on cloud-only files),
/// `GetFileAttributesW` reads from the local cached attributes maintained by the
/// cloud filter driver — no network I/O, no file recall, returns in microseconds.
///
/// Use this instead of `path.exists()` anywhere on the UI thread or hot paths.
pub fn fast_path_exists(path: &Path) -> bool {
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};

    let path_wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let attrs = unsafe { GetFileAttributesW(windows::core::PCWSTR(path_wide.as_ptr())) };
    attrs != INVALID_FILE_ATTRIBUTES
}

/// Fast directory check using GetFileAttributesW.
///
/// Returns true if the path exists AND is a directory.
/// Same performance characteristics as `fast_path_exists` — no OneDrive file recall.
pub fn fast_is_dir(path: &Path) -> bool {
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};

    let path_wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let attrs = unsafe { GetFileAttributesW(windows::core::PCWSTR(path_wide.as_ptr())) };
    if attrs == INVALID_FILE_ATTRIBUTES {
        return false;
    }
    (attrs & 0x10) != 0 // FILE_ATTRIBUTE_DIRECTORY
}

/// Uncached cloud attribute check via Win32 API
fn check_cloud_attributes_uncached(path: &Path) -> bool {
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};

    let path_wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let attrs = unsafe { GetFileAttributesW(windows::core::PCWSTR(path_wide.as_ptr())) };
    if attrs == INVALID_FILE_ATTRIBUTES {
        return false;
    }
    has_cloud_attributes(attrs)
}

/// Check if a file is currently open in any application.
/// Uses a simple heuristic: tries to open the file with exclusive access.
/// If it fails, the file is likely open in another application.
pub fn is_file_open(path: &Path) -> bool {
    use std::fs::OpenOptions;
    use std::io;

    // For files, try to open with exclusive access
    // If another process has it open, this will fail
    match OpenOptions::new()
        .read(true)
        .write(true)
        .create(false)
        .open(path)
    {
        Ok(_) => {
            // Successfully opened, so it's not being used by another process
            false
        }
        Err(e) => {
            // Failed to open - likely because it's in use
            // Check specifically for "file in use" errors
            e.kind() == io::ErrorKind::PermissionDenied
                || e.raw_os_error().map_or(false, |code| {
                    // Windows error codes for "file in use":
                    // ERROR_SHARING_VIOLATION (32), ERROR_LOCK_VIOLATION (33)
                    code == 32 || code == 33
                })
        }
    }
}

/// Determine sync status from file attributes.
/// Only assigns sync status when `is_onedrive` is true (path confirmed to be under a cloud root).
/// Note: `is_onedrive` should already account for alternate mount points via
/// `path_has_cloud_attributes()` at the directory level. We intentionally do NOT
/// fall back to per-file cloud attributes here, because files copied FROM OneDrive
/// retain their cloud attributes even in non-OneDrive locations.
pub fn get_sync_status(attrs: u32, is_onedrive: bool) -> SyncStatus {
    if !is_onedrive {
        return SyncStatus::None;
    }

    // Syncing: File is being actively synced (highest priority)
    if (attrs & FILE_ATTRIBUTE_RECALL_ON_OPEN) != 0 {
        return SyncStatus::Syncing;
    }

    // Cloud Only: File needs to be downloaded (placeholder)
    if (attrs & FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0 || (attrs & FILE_ATTRIBUTE_OFFLINE) != 0
    {
        return SyncStatus::CloudOnly;
    }

    // Pinned: Always keep on device
    if (attrs & FILE_ATTRIBUTE_PINNED) != 0 {
        return SyncStatus::Pinned;
    }

    // LocallyAvailable: Downloaded but not pinned
    SyncStatus::LocallyAvailable
}

/// Check if a file is available locally (not cloud-only).
/// Returns true if the file data is on disk, false if it needs download.
pub fn is_locally_available(path: &Path) -> bool {
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};

    let path_wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let attrs = unsafe { GetFileAttributesW(windows::core::PCWSTR(path_wide.as_ptr())) };

    if attrs == INVALID_FILE_ATTRIBUTES {
        return false; // File doesn't exist or error
    }

    // Cloud-only indicators: need to be recalled/downloaded
    let is_cloud_only = (attrs & FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0
        || (attrs & FILE_ATTRIBUTE_OFFLINE) != 0;

    !is_cloud_only
}

/// Result of a timeout-protected I/O operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoTimeoutResult<T> {
    /// Operation completed successfully within timeout
    Ok(T),
    /// Operation timed out - file is likely cloud-only or network is slow
    Timeout,
    /// Operation failed with an error
    Err(std::io::ErrorKind),
}

impl<T> IoTimeoutResult<T> {
    /// Convert to Option, treating Timeout as None
    pub fn ok(self) -> Option<T> {
        match self {
            IoTimeoutResult::Ok(v) => Some(v),
            _ => None,
        }
    }

    /// Returns true if the operation timed out
    pub fn is_timeout(&self) -> bool {
        matches!(self, IoTimeoutResult::Timeout)
    }

    /// Returns true if the operation succeeded
    pub fn is_ok(&self) -> bool {
        matches!(self, IoTimeoutResult::Ok(_))
    }
}

/// Get file metadata with timeout protection.
/// CRITICAL for OneDrive: prevents indefinite blocking on cloud-only files.
///
/// Returns:
/// - `Ok(metadata)` if successful within timeout
/// - `Timeout` if operation takes longer than timeout_ms
/// - `Err(kind)` if operation fails
pub fn metadata_with_timeout(
    path: &Path,
    timeout_ms: u64,
) -> IoTimeoutResult<std::fs::Metadata> {
    // Fast path: check if it's even a OneDrive path
    if !is_onedrive_path(path) {
        // Not OneDrive - use regular metadata (should be fast)
        match std::fs::metadata(path) {
            Ok(m) => return IoTimeoutResult::Ok(m),
            Err(e) => return IoTimeoutResult::Err(e.kind()),
        }
    }

    // Check if app is minimized - return timeout immediately to avoid spawning threads
    if is_app_minimized() {
        eprintln!("[ONEDRIVE] App minimized - skipping metadata for {:?}", path);
        return IoTimeoutResult::Timeout;
    }

    // Check concurrent thread limit BEFORE spawning
    let current_threads = ACTIVE_TIMEOUT_THREADS.load(Ordering::SeqCst);
    if current_threads >= MAX_CONCURRENT_TIMEOUT_THREADS {
        eprintln!(
            "[ONEDRIVE] Thread limit reached ({}/{}), rejecting metadata() for {:?}",
            current_threads, MAX_CONCURRENT_TIMEOUT_THREADS, path
        );
        return IoTimeoutResult::Timeout;
    }

    // Adjust timeout if minimized (use shorter timeout)
    let effective_timeout = if is_app_minimized() {
        ONEDRIVE_METADATA_TIMEOUT_MINIMIZED_MS
    } else {
        timeout_ms
    };

    // Increment active thread counter
    let active_before = ACTIVE_TIMEOUT_THREADS.fetch_add(1, Ordering::SeqCst);
    eprintln!("[ONEDRIVE] Active timeout threads: {} -> {}", active_before, active_before + 1);

    // OneDrive path: use timeout protection
    let path_buf = path.to_path_buf();
    let path_for_log = path_buf.clone();
    let timeout = Duration::from_millis(effective_timeout);
    let start = Instant::now();

    // Spawn a thread to do the blocking I/O
    let handle = std::thread::spawn(move || std::fs::metadata(&path_buf));

    // Poll with timeout
    let result = loop {
        // Check if app was minimized during operation
        if is_app_minimized() {
            eprintln!("[ONEDRIVE] App minimized during operation - aborting metadata for {:?}", path_for_log);
            break IoTimeoutResult::Timeout;
        }

        if start.elapsed() >= timeout {
            // Timeout reached - detach thread (it will eventually complete or fail)
            eprintln!(
                "[ONEDRIVE TIMEOUT] metadata() exceeded {}ms for {:?}",
                effective_timeout,
                path_for_log
            );
            break IoTimeoutResult::Timeout;
        }

        // Check if thread is done (non-blocking)
        if handle.is_finished() {
            break match handle.join() {
                Ok(Ok(metadata)) => IoTimeoutResult::Ok(metadata),
                Ok(Err(e)) => IoTimeoutResult::Err(e.kind()),
                Err(_) => IoTimeoutResult::Err(std::io::ErrorKind::Other),
            };
        }

        // Small sleep to prevent busy-waiting
        std::thread::sleep(Duration::from_millis(1));
    };

    // Decrement active thread counter
    let active_after = ACTIVE_TIMEOUT_THREADS.fetch_sub(1, Ordering::SeqCst);
    eprintln!("[ONEDRIVE] Active timeout threads: {} -> {}", active_after, active_after - 1);

    result
}

/// Check if path exists with timeout protection.
/// CRITICAL for OneDrive: `path.exists()` can trigger download of cloud-only files.
///
/// Returns:
/// - `Ok(true/false)` if successful within timeout
/// - `Timeout` if operation takes longer than timeout_ms
pub fn exists_with_timeout(path: &Path, timeout_ms: u64) -> IoTimeoutResult<bool> {
    // Fast path: check if it's even a OneDrive path
    if !is_onedrive_path(path) {
        // Not OneDrive - use GetFileAttributesW (faster than path.exists() which uses CreateFileW)
        return IoTimeoutResult::Ok(fast_path_exists(path));
    }

    // Check if app is minimized - return timeout immediately
    if is_app_minimized() {
        eprintln!("[ONEDRIVE] App minimized - skipping exists for {:?}", path);
        return IoTimeoutResult::Timeout;
    }

    // Check concurrent thread limit BEFORE spawning
    let current_threads = ACTIVE_TIMEOUT_THREADS.load(Ordering::SeqCst);
    if current_threads >= MAX_CONCURRENT_TIMEOUT_THREADS {
        eprintln!(
            "[ONEDRIVE] Thread limit reached ({}/{}), rejecting exists() for {:?}",
            current_threads, MAX_CONCURRENT_TIMEOUT_THREADS, path
        );
        return IoTimeoutResult::Timeout;
    }

    // Adjust timeout if minimized
    let effective_timeout = if is_app_minimized() {
        ONEDRIVE_METADATA_TIMEOUT_MINIMIZED_MS
    } else {
        timeout_ms
    };

    // Increment active thread counter
    let active_before = ACTIVE_TIMEOUT_THREADS.fetch_add(1, Ordering::SeqCst);
    eprintln!("[ONEDRIVE] Active timeout threads: {} -> {}", active_before, active_before + 1);

    // OneDrive path: use timeout protection
    let path_buf = path.to_path_buf();
    let path_for_log = path_buf.clone();
    let timeout = Duration::from_millis(effective_timeout);
    let start = Instant::now();

    // Spawn a thread to do the blocking I/O
    let handle = std::thread::spawn(move || path_buf.exists());

    // Poll with timeout
    let result = loop {
        // Check if app was minimized during operation
        if is_app_minimized() {
            eprintln!("[ONEDRIVE] App minimized during operation - aborting exists for {:?}", path_for_log);
            break IoTimeoutResult::Timeout;
        }

        if start.elapsed() >= timeout {
            eprintln!(
                "[ONEDRIVE TIMEOUT] exists() exceeded {}ms for {:?}",
                effective_timeout,
                path_for_log
            );
            break IoTimeoutResult::Timeout;
        }

        if handle.is_finished() {
            break match handle.join() {
                Ok(exists) => IoTimeoutResult::Ok(exists),
                Err(_) => IoTimeoutResult::Err(std::io::ErrorKind::Other),
            };
        }

        std::thread::sleep(Duration::from_millis(1));
    };

    // Decrement active thread counter
    let active_after = ACTIVE_TIMEOUT_THREADS.fetch_sub(1, Ordering::SeqCst);
    eprintln!("[ONEDRIVE] Active timeout threads: {} -> {}", active_after, active_after - 1);

    result
}

/// Convenience function: Get metadata with default OneDrive timeout (100ms)
pub fn onedrive_metadata(path: &Path) -> IoTimeoutResult<std::fs::Metadata> {
    metadata_with_timeout(path, ONEDRIVE_METADATA_TIMEOUT_MS)
}

/// Convenience function: Check exists with default OneDrive timeout (50ms)
pub fn onedrive_exists(path: &Path) -> IoTimeoutResult<bool> {
    exists_with_timeout(path, ONEDRIVE_EXISTS_TIMEOUT_MS)
}

/// Safely check if a file is locally available (not cloud-only).
///
/// Uses `GetFileAttributesW` which reads cached attributes from the cloud filter
/// driver — no file handle, no network I/O, returns in microseconds.
///
/// The cloud filter driver (e.g., OneDrive) is the authoritative source for
/// placeholder status. The Files app (and Windows Explorer) trust these attributes
/// without secondary `metadata()` verification. We follow the same pattern.
///
/// Previously this function did a secondary `std::fs::metadata()` call with timeout
/// to "double-check" availability, but this was unnecessary and expensive
/// (spawned a timeout thread per file). The `GetFileAttributesW` result is sufficient.
pub fn is_locally_available_safe(path: &Path) -> bool {
    is_locally_available(path)
}

/// Result type for directory enumeration with timeout
pub type DirectoryEntries = Vec<(String, u32, u64, u64)>; // (name, attributes, size, modified)

/// Reads a directory with timeout protection for OneDrive.
/// This is CRITICAL because FindFirstFileW can block for 30-60 seconds on OneDrive folders.
///
/// Returns:
/// - `Ok(entries)` if successful within timeout
/// - `Timeout` if enumeration takes longer than timeout_ms
pub fn read_directory_with_timeout(
    path: &Path,
    timeout_ms: u64,
) -> IoTimeoutResult<DirectoryEntries> {
    // Fast path: not OneDrive - use regular reading (should be fast)
    if !is_onedrive_path(path) {
        return match read_directory_internal(path) {
            Ok(entries) => IoTimeoutResult::Ok(entries),
            Err(_) => IoTimeoutResult::Err(std::io::ErrorKind::Other),
        };
    }

    // Check if app is minimized - return timeout immediately
    if is_app_minimized() {
        eprintln!("[ONEDRIVE] App minimized - skipping read_directory for {:?}", path);
        return IoTimeoutResult::Timeout;
    }

    // Check concurrent thread limit BEFORE spawning
    let current_threads = ACTIVE_TIMEOUT_THREADS.load(Ordering::SeqCst);
    if current_threads >= MAX_CONCURRENT_TIMEOUT_THREADS {
        eprintln!(
            "[ONEDRIVE] Thread limit reached ({}/{}), rejecting read_directory() for {:?}",
            current_threads, MAX_CONCURRENT_TIMEOUT_THREADS, path
        );
        return IoTimeoutResult::Timeout;
    }

    // Adjust timeout if minimized
    let effective_timeout = if is_app_minimized() {
        timeout_ms / 2
    } else {
        timeout_ms
    };

    // Increment active thread counter
    let active_before = ACTIVE_TIMEOUT_THREADS.fetch_add(1, Ordering::SeqCst);
    eprintln!("[ONEDRIVE] Active timeout threads: {} -> {}", active_before, active_before + 1);

    // OneDrive path: use timeout protection
    let path_buf = path.to_path_buf();
    let path_for_log = path_buf.clone();
    let timeout = Duration::from_millis(effective_timeout);
    let start = Instant::now();

    // Spawn a thread to do the blocking directory enumeration
    let handle = std::thread::spawn(move || read_directory_internal(&path_buf));

    // Poll with timeout
    let result = loop {
        // Check if app was minimized during operation
        if is_app_minimized() {
            eprintln!("[ONEDRIVE] App minimized during operation - aborting read_directory for {:?}", path_for_log);
            break IoTimeoutResult::Timeout;
        }

        if start.elapsed() >= timeout {
            eprintln!(
                "[ONEDRIVE TIMEOUT] read_directory() exceeded {}ms for {:?}",
                effective_timeout,
                path_for_log
            );
            break IoTimeoutResult::Timeout;
        }

        if handle.is_finished() {
            break match handle.join() {
                Ok(Ok(entries)) => IoTimeoutResult::Ok(entries),
                Ok(Err(_)) => IoTimeoutResult::Err(std::io::ErrorKind::Other),
                Err(_) => IoTimeoutResult::Err(std::io::ErrorKind::Other),
            };
        }

        std::thread::sleep(Duration::from_millis(5)); // 5ms polling interval
    };

    // Decrement active thread counter
    let active_after = ACTIVE_TIMEOUT_THREADS.fetch_sub(1, Ordering::SeqCst);
    eprintln!("[ONEDRIVE] Active timeout threads: {} -> {}", active_after, active_after - 1);

    result
}

/// Internal directory reading function using Win32 APIs
fn read_directory_internal(path: &Path) -> Result<DirectoryEntries, std::io::Error> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        FindClose, FindFirstFileW, FindNextFileW, WIN32_FIND_DATAW,
    };

    let search_path = if path.to_string_lossy().ends_with('\\') {
        format!("{}*", path.display())
    } else {
        format!("{}\\*", path.display())
    };

    let wide_path: Vec<u16> = search_path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut find_data = WIN32_FIND_DATAW::default();
    let mut entries = Vec::new();

    unsafe {
        let handle = FindFirstFileW(PCWSTR(wide_path.as_ptr()), &mut find_data)?;

        loop {
            // Extract filename
            let len = find_data
                .cFileName
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(find_data.cFileName.len());
            let filename = OsString::from_wide(&find_data.cFileName[0..len])
                .to_string_lossy()
                .into_owned();

            if filename != "." && filename != ".." {
                let attrs = find_data.dwFileAttributes;
                let is_dir = (attrs & 0x10) != 0;

                let size = if is_dir {
                    0
                } else {
                    ((find_data.nFileSizeHigh as u64) << 32) | (find_data.nFileSizeLow as u64)
                };

                let ft = find_data.ftLastWriteTime;
                let windows_ticks = ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
                let modified = if windows_ticks > 116444736000000000 {
                    (windows_ticks - 116444736000000000) / 10_000_000
                } else {
                    0
                };

                entries.push((filename, attrs, size, modified));
            }

            if FindNextFileW(handle, &mut find_data).is_err() {
                break;
            }
        }

        let _ = FindClose(handle);
    }

    Ok(entries)
}

/// Convenience function: Read directory with default OneDrive timeout (5s)
pub fn onedrive_read_directory(path: &Path) -> IoTimeoutResult<DirectoryEntries> {
    read_directory_with_timeout(path, ONEDRIVE_DIR_ENUM_TIMEOUT_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_status_none_when_not_onedrive() {
        assert_eq!(get_sync_status(0, false), SyncStatus::None);
    }

    #[test]
    fn test_sync_status_cloud_only() {
        assert_eq!(
            get_sync_status(FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, true),
            SyncStatus::CloudOnly
        );
        assert_eq!(
            get_sync_status(FILE_ATTRIBUTE_OFFLINE, true),
            SyncStatus::CloudOnly
        );
    }

    #[test]
    fn test_sync_status_pinned() {
        assert_eq!(
            get_sync_status(FILE_ATTRIBUTE_PINNED, true),
            SyncStatus::Pinned
        );
    }

    #[test]
    fn test_sync_status_syncing() {
        assert_eq!(
            get_sync_status(FILE_ATTRIBUTE_RECALL_ON_OPEN, true),
            SyncStatus::Syncing
        );
    }

    #[test]
    fn test_sync_status_locally_available() {
        assert_eq!(get_sync_status(0, true), SyncStatus::LocallyAvailable);
    }

    #[test]
    fn test_cloud_flags_without_known_root_returns_none() {
        // Files with cloud attributes outside OneDrive paths should NOT get sync status.
        // This prevents false positives for files copied from OneDrive to other locations.
        assert_eq!(
            get_sync_status(FILE_ATTRIBUTE_PINNED, false),
            SyncStatus::None
        );
        assert_eq!(
            get_sync_status(FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, false),
            SyncStatus::None
        );
    }
}
