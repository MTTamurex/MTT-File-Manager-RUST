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
const FILE_ATTRIBUTE_PINNED: u32 = 0x00080000;
const FILE_ATTRIBUTE_OFFLINE: u32 = 0x00001000;

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
    ONEDRIVE_ROOTS.get()
        .map(|roots| roots.iter().any(|r| path_lower.starts_with(r)))
        .unwrap_or(false)
}

/// Determine sync status from file attributes.
/// Only meaningful when `is_onedrive` is true.
pub fn get_sync_status(attrs: u32, is_onedrive: bool) -> SyncStatus {
    if !is_onedrive {
        return SyncStatus::None;
    }
    
    // Cloud Only: File needs to be downloaded (placeholder)
    if (attrs & FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0 
       || (attrs & FILE_ATTRIBUTE_OFFLINE) != 0 {
        return SyncStatus::CloudOnly;
    }
    
    // Pinned: Always keep on device
    if (attrs & FILE_ATTRIBUTE_PINNED) != 0 {
        return SyncStatus::Pinned;
    }
    
    // LocallyAvailable: Downloaded but not pinned
    SyncStatus::LocallyAvailable
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_status_none_when_not_onedrive() {
        assert_eq!(get_sync_status(0, false), SyncStatus::None);
        assert_eq!(get_sync_status(FILE_ATTRIBUTE_PINNED, false), SyncStatus::None);
    }

    #[test]
    fn test_sync_status_cloud_only() {
        assert_eq!(get_sync_status(FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, true), SyncStatus::CloudOnly);
        assert_eq!(get_sync_status(FILE_ATTRIBUTE_OFFLINE, true), SyncStatus::CloudOnly);
    }

    #[test]
    fn test_sync_status_pinned() {
        assert_eq!(get_sync_status(FILE_ATTRIBUTE_PINNED, true), SyncStatus::Pinned);
    }

    #[test]
    fn test_sync_status_locally_available() {
        assert_eq!(get_sync_status(0, true), SyncStatus::LocallyAvailable);
    }
}
