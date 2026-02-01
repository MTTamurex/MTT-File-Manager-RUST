//! OneDrive sync status detection utilities.
//!
//! This module provides functions to detect if a path is within a OneDrive
//! folder and to parse file attributes into sync status values.

use std::path::Path;
use std::sync::OnceLock;

use crate::domain::file_entry::SyncStatus;

// Cached OneDrive root paths (initialized once at startup)
static ONEDRIVE_ROOTS: OnceLock<Vec<String>> = OnceLock::new();

// Windows file attribute constants for cloud files (undocumented but well-known)
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x00400000;
const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x00040000; // File is being synced
const FILE_ATTRIBUTE_PINNED: u32 = 0x00080000;
const FILE_ATTRIBUTE_OFFLINE: u32 = 0x00001000;

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
pub fn path_has_cloud_attributes(path: &Path) -> bool {
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
/// Falls back to attribute-based detection if the path is not under a known OneDrive root.
pub fn get_sync_status(attrs: u32, is_onedrive: bool) -> SyncStatus {
    let is_cloud_file = is_onedrive || has_cloud_attributes(attrs);

    if !is_cloud_file {
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
    fn test_cloud_flags_without_known_root() {
        assert_eq!(
            get_sync_status(FILE_ATTRIBUTE_PINNED, false),
            SyncStatus::Pinned
        );
        assert_eq!(
            get_sync_status(FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, false),
            SyncStatus::CloudOnly
        );
    }
}
