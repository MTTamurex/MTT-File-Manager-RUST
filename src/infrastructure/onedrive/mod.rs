//! OneDrive sync status detection utilities.
//!
//! This module provides functions to detect if a path is within a OneDrive
//! folder and to parse file attributes into sync status values.
//!
//! PERFORMANCE CRITICAL: All I/O operations on OneDrive files use timeout-based
//! wrappers to prevent indefinite blocking on cloud-only files.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::domain::file_entry::SyncStatus;
mod attributes;
mod directory_enum;
mod path_detection;
mod pin_state;
mod timeout_ops;

// Base worker count for timeout-protected OneDrive I/O.
const ONEDRIVE_IO_BASE_WORKERS: usize = 4;
// Temporary overflow workers to avoid starvation when base workers are stuck.
const ONEDRIVE_IO_MAX_OVERFLOW_WORKERS: usize = 24;
// Maximum concurrent timeout requests waiting on results.
const MAX_CONCURRENT_TIMEOUT_THREADS: u64 = 32;

// Counter of active timeout threads for monitoring and limiting
static ACTIVE_TIMEOUT_THREADS: AtomicU64 = AtomicU64::new(0);

// Global flag indicating if app is minimized (for operation cancellation)
static APP_MINIMIZED: AtomicBool = AtomicBool::new(false);

// Timeout configurations
const ONEDRIVE_METADATA_TIMEOUT_MINIMIZED_MS: u64 = 50;

// Cached OneDrive root paths (initialized once at startup)
static ONEDRIVE_ROOTS: OnceLock<Vec<String>> = OnceLock::new();
static ONEDRIVE_IO_POOL: OnceLock<OneDriveIoPool> = OnceLock::new();

type IoJob = Box<dyn FnOnce() + Send + 'static>;

struct OneDriveIoPool {
    sender: mpsc::SyncSender<IoJob>,
    receiver: Arc<Mutex<mpsc::Receiver<IoJob>>>,
    active_workers: Arc<AtomicUsize>,
    overflow_workers: Arc<AtomicUsize>,
    max_overflow_workers: usize,
}

impl OneDriveIoPool {
    fn new(worker_count: usize, max_overflow_workers: usize) -> Self {
        // Keep queue small to avoid long backlogs behind blocked I/O.
        let queue_capacity = worker_count.max(4);
        let (sender, receiver) = mpsc::sync_channel::<IoJob>(queue_capacity);
        let receiver = Arc::new(Mutex::new(receiver));
        let active_workers = Arc::new(AtomicUsize::new(0));
        let overflow_workers = Arc::new(AtomicUsize::new(0));

        for worker_id in 0..worker_count {
            let receiver_clone = Arc::clone(&receiver);
            let active_workers_clone = Arc::clone(&active_workers);
            let _ = std::thread::Builder::new()
                .name(format!("onedrive-io-{}", worker_id))
                .spawn(move || loop {
                    let recv_result = match receiver_clone.lock() {
                        Ok(rx) => rx.recv(),
                        Err(_) => return,
                    };

                    match recv_result {
                        Ok(job) => {
                            active_workers_clone.fetch_add(1, Ordering::SeqCst);
                            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                            active_workers_clone.fetch_sub(1, Ordering::SeqCst);
                        }
                        Err(_) => break,
                    }
                });
        }

        Self {
            sender,
            receiver,
            active_workers,
            overflow_workers,
            max_overflow_workers,
        }
    }

    fn execute<F>(&self, job: F) -> bool
    where
        F: FnOnce() + Send + 'static,
    {
        let queued = match self.sender.try_send(Box::new(job)) {
            Ok(()) => true,
            Err(mpsc::TrySendError::Full(job)) => {
                // Queue is full while workers are likely blocked.
                // Run job directly on an overflow worker to keep progress.
                return self.try_spawn_overflow_worker_with_job(job);
            }
            Err(mpsc::TrySendError::Disconnected(_)) => return false,
        };

        if queued {
            let base_saturated =
                self.active_workers.load(Ordering::Acquire) >= ONEDRIVE_IO_BASE_WORKERS;
            if base_saturated {
                let _ = self.try_spawn_overflow_worker();
            }
        }

        true
    }

    fn try_acquire_overflow_slot(&self) -> bool {
        self.overflow_workers
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                if current < self.max_overflow_workers {
                    Some(current + 1)
                } else {
                    None
                }
            })
            .is_ok()
    }

    fn try_spawn_overflow_worker(&self) -> bool {
        if !self.try_acquire_overflow_slot() {
            return false;
        }

        let receiver = Arc::clone(&self.receiver);
        let active_workers = Arc::clone(&self.active_workers);
        let overflow_workers = Arc::clone(&self.overflow_workers);

        let spawn_result = std::thread::Builder::new()
            .name("onedrive-io-overflow".to_string())
            .spawn(move || {
                let recv_result = match receiver.lock() {
                    Ok(rx) => rx.recv_timeout(Duration::from_millis(10)),
                    Err(_) => Err(mpsc::RecvTimeoutError::Disconnected),
                };

                if let Ok(job) = recv_result {
                    active_workers.fetch_add(1, Ordering::SeqCst);
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                    active_workers.fetch_sub(1, Ordering::SeqCst);
                }

                overflow_workers.fetch_sub(1, Ordering::SeqCst);
            });

        if spawn_result.is_err() {
            self.overflow_workers.fetch_sub(1, Ordering::SeqCst);
            return false;
        }

        true
    }

    fn try_spawn_overflow_worker_with_job(&self, job: IoJob) -> bool {
        if !self.try_acquire_overflow_slot() {
            return false;
        }

        let active_workers = Arc::clone(&self.active_workers);
        let overflow_workers = Arc::clone(&self.overflow_workers);

        let spawn_result = std::thread::Builder::new()
            .name("onedrive-io-overflow-direct".to_string())
            .spawn(move || {
                active_workers.fetch_add(1, Ordering::SeqCst);
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                active_workers.fetch_sub(1, Ordering::SeqCst);
                overflow_workers.fetch_sub(1, Ordering::SeqCst);
            });

        if spawn_result.is_err() {
            self.overflow_workers.fetch_sub(1, Ordering::SeqCst);
            return false;
        }

        true
    }
}

fn onedrive_io_pool() -> &'static OneDriveIoPool {
    ONEDRIVE_IO_POOL.get_or_init(|| {
        OneDriveIoPool::new(ONEDRIVE_IO_BASE_WORKERS, ONEDRIVE_IO_MAX_OVERFLOW_WORKERS)
    })
}

/// Set the minimized state of the application.
/// When minimized, timeout operations are cancelled more aggressively.
pub fn set_app_minimized(minimized: bool) {
    APP_MINIMIZED.store(minimized, Ordering::SeqCst);
    log::debug!(
        "[ONEDRIVE LIFECYCLE] App minimized state changed: {}",
        minimized
    );
    if minimized {
        log::debug!(
            "[ONEDRIVE LIFECYCLE] Active timeout threads at minimize: {}",
            ACTIVE_TIMEOUT_THREADS.load(Ordering::SeqCst)
        );
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
    attributes::has_cloud_attributes(attrs)
}

/// Initialize OneDrive root paths from environment variables.
/// Should be called once at application startup.
pub fn init_onedrive_paths() {
    path_detection::init_onedrive_paths();
}

/// Check if a path is within a OneDrive folder.
/// Uses cached roots from environment variables.
pub fn is_onedrive_path(path: &Path) -> bool {
    path_detection::is_onedrive_path(path)
}

/// Fallback detection using file attributes for cases where the OneDrive root
/// isn't covered by environment variables (e.g., secondary business accounts).
///
/// Calls GetFileAttributesW directly (fast cached filesystem call, no disk I/O).
/// NOT cached per drive letter — different paths on the same drive can have
/// different cloud attributes (e.g., C:\Users\Docs vs C:\Users\OneDrive).
pub fn path_has_cloud_attributes(path: &Path) -> bool {
    attributes::path_has_cloud_attributes(path)
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
    attributes::fast_path_exists(path)
}

/// Fast directory check using GetFileAttributesW.
///
/// Returns true if the path exists AND is a directory.
/// Same performance characteristics as `fast_path_exists` — no OneDrive file recall.
pub fn fast_is_dir(path: &Path) -> bool {
    attributes::fast_is_dir(path)
}

/// Check if a file is currently open in any application.
/// Uses a simple heuristic: tries to open the file with exclusive access.
/// If it fails, the file is likely open in another application.
pub fn is_file_open(path: &Path) -> bool {
    attributes::is_file_open(path)
}

/// Determine sync status from file attributes.
/// Only assigns sync status when `is_onedrive` is true (path confirmed to be under a cloud root).
/// Note: `is_onedrive` should already account for alternate mount points via
/// `path_has_cloud_attributes()` at the directory level. We intentionally do NOT
/// fall back to per-file cloud attributes here, because files copied FROM OneDrive
/// retain their cloud attributes even in non-OneDrive locations.
pub fn get_sync_status(attrs: u32, is_onedrive: bool) -> SyncStatus {
    attributes::get_sync_status(attrs, is_onedrive)
}

/// Check if a file is available locally (not cloud-only).
/// Returns true if the file data is on disk, false if it needs download.
pub fn is_locally_available(path: &Path) -> bool {
    attributes::is_locally_available(path)
}

pub use pin_state::PinCommand;

/// Apply OneDrive pin-state operation using Windows `attrib` flags.
///
/// - [`PinCommand::AlwaysKeepOnDevice`](src/infrastructure/onedrive/pin_state.rs:7) => `+P -U`
/// - [`PinCommand::FreeUpSpace`](src/infrastructure/onedrive/pin_state.rs:8) => `+U -P`
pub fn set_pin_state(path: &Path, command: PinCommand) -> std::io::Result<()> {
    pin_state::set_pin_state(path, command)
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
pub fn metadata_with_timeout(path: &Path, timeout_ms: u64) -> IoTimeoutResult<std::fs::Metadata> {
    timeout_ops::metadata_with_timeout(path, timeout_ms)
}

/// Check if path exists with timeout protection.
/// CRITICAL for OneDrive: `path.exists()` can trigger download of cloud-only files.
///
/// Returns:
/// - `Ok(true/false)` if successful within timeout
/// - `Timeout` if operation takes longer than timeout_ms
pub fn exists_with_timeout(path: &Path, timeout_ms: u64) -> IoTimeoutResult<bool> {
    timeout_ops::exists_with_timeout(path, timeout_ms)
}

/// Convenience function: Get metadata with default OneDrive timeout (100ms)
pub fn onedrive_metadata(path: &Path) -> IoTimeoutResult<std::fs::Metadata> {
    timeout_ops::onedrive_metadata(path)
}

/// Convenience function: Check exists with default OneDrive timeout (50ms)
pub fn onedrive_exists(path: &Path) -> IoTimeoutResult<bool> {
    timeout_ops::onedrive_exists(path)
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
    directory_enum::read_directory_with_timeout(path, timeout_ms)
}

/// Convenience function: Read directory with default OneDrive timeout (5s)
pub fn onedrive_read_directory(path: &Path) -> IoTimeoutResult<DirectoryEntries> {
    directory_enum::onedrive_read_directory(path)
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
