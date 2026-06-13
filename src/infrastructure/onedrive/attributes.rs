use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use crate::domain::file_entry::SyncStatus;

pub(super) fn has_cloud_attributes(attrs: u32) -> bool {
    (attrs & super::FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0
        || (attrs & super::FILE_ATTRIBUTE_RECALL_ON_OPEN) != 0
        || (attrs & super::FILE_ATTRIBUTE_PINNED) != 0
        || (attrs & super::FILE_ATTRIBUTE_UNPINNED) != 0
        || (attrs & super::FILE_ATTRIBUTE_OFFLINE) != 0
}

pub(super) fn path_has_cloud_attributes(path: &Path) -> bool {
    check_cloud_attributes_uncached(path)
}

pub(super) fn fast_path_exists(path: &Path) -> bool {
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};

    let path_wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let attrs = unsafe { GetFileAttributesW(windows::core::PCWSTR(path_wide.as_ptr())) };
    attrs != INVALID_FILE_ATTRIBUTES
}

pub(super) fn fast_is_dir(path: &Path) -> bool {
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};

    let path_wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let attrs = unsafe { GetFileAttributesW(windows::core::PCWSTR(path_wide.as_ptr())) };
    if attrs == INVALID_FILE_ATTRIBUTES {
        return false;
    }
    (attrs & 0x10) != 0
}

pub(super) fn is_file_open(path: &Path) -> bool {
    use std::fs::OpenOptions;
    use std::io;

    match OpenOptions::new()
        .read(true)
        .write(true)
        .create(false)
        .open(path)
    {
        Ok(_) => false,
        Err(e) => {
            e.kind() == io::ErrorKind::PermissionDenied
                || e.raw_os_error()
                    .is_some_and(|code| code == 32 || code == 33)
        }
    }
}

pub(super) fn get_sync_status(attrs: u32, is_onedrive: bool) -> SyncStatus {
    if !is_onedrive {
        return SyncStatus::None;
    }

    if (attrs & super::FILE_ATTRIBUTE_RECALL_ON_OPEN) != 0 {
        return SyncStatus::Syncing;
    }

    if (attrs & super::FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0
        || (attrs & super::FILE_ATTRIBUTE_OFFLINE) != 0
    {
        return SyncStatus::CloudOnly;
    }

    if (attrs & super::FILE_ATTRIBUTE_PINNED) != 0 {
        return SyncStatus::Pinned;
    }

    // UNPINNED = user selected "Free up space" but dehydration hasn't completed yet.
    // The file data is still local, but the intent is cloud-only.
    // Show CloudOnly immediately (matches Windows Explorer behavior).
    if (attrs & super::FILE_ATTRIBUTE_UNPINNED) != 0 {
        return SyncStatus::CloudOnly;
    }

    SyncStatus::LocallyAvailable
}

pub(super) fn sync_status_for_cloud_path(path: &Path) -> Option<SyncStatus> {
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};

    let path_wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let attrs = unsafe { GetFileAttributesW(windows::core::PCWSTR(path_wide.as_ptr())) };
    if attrs == INVALID_FILE_ATTRIBUTES {
        return None;
    }

    Some(get_sync_status(attrs, true))
}

pub(super) fn is_locally_available(path: &Path) -> bool {
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};

    let path_wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let attrs = unsafe { GetFileAttributesW(windows::core::PCWSTR(path_wide.as_ptr())) };

    if attrs == INVALID_FILE_ATTRIBUTES {
        return false;
    }

    let is_cloud_only = (attrs & super::FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0
        || (attrs & super::FILE_ATTRIBUTE_OFFLINE) != 0;

    !is_cloud_only
}

fn check_cloud_attributes_uncached(path: &Path) -> bool {
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};

    let path_wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let attrs = unsafe { GetFileAttributesW(windows::core::PCWSTR(path_wide.as_ptr())) };
    if attrs == INVALID_FILE_ATTRIBUTES {
        return false;
    }
    has_cloud_attributes(attrs)
}
