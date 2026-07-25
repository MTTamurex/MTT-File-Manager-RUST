use std::collections::HashMap;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::Storage::Vhd::*;

use super::owned_handle::OwnedHandle;

/// Mounts an ISO file programmatically.
/// This will trigger a volume arrival event (WM_DEVICECHANGE) in the system.
/// SAFETY: Interacts with Windows Virtual Disk API.
pub fn mount_iso(path: &Path) -> Result<()> {
    unsafe {
        let path_wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let storage_type = VIRTUAL_STORAGE_TYPE {
            DeviceId: VIRTUAL_STORAGE_TYPE_DEVICE_ISO,
            VendorId: VIRTUAL_STORAGE_TYPE_VENDOR_MICROSOFT,
        };

        let mut handle = HANDLE::default();

        // 1. Open the virtual disk
        // Version 1 is often more compatible for simple ISO opening
        let open_params = OPEN_VIRTUAL_DISK_PARAMETERS {
            Version: OPEN_VIRTUAL_DISK_VERSION_1,
            ..Default::default()
        };

        log::debug!("[ISO] Opening virtual disk (V1): {:?}", path);
        OpenVirtualDisk(
            &storage_type,
            PCWSTR(path_wide.as_ptr()),
            VIRTUAL_DISK_ACCESS_ATTACH_RO,
            OPEN_VIRTUAL_DISK_FLAG_NONE,
            Some(&open_params),
            &mut handle,
        )
        .ok()
        .map_err(|e| {
            log::error!("[ISO] OpenVirtualDisk failed: {:?}", e);
            e
        })?;

        // 2. Attach the virtual disk
        let attach_params = ATTACH_VIRTUAL_DISK_PARAMETERS {
            Version: ATTACH_VIRTUAL_DISK_VERSION_1,
            ..Default::default()
        };

        log::debug!("[ISO] Attaching virtual disk handle: {:?}", handle);
        // PERMANENT_LIFETIME keeps the mount active after CloseHandle
        AttachVirtualDisk(
            handle,
            None,
            ATTACH_VIRTUAL_DISK_FLAG_READ_ONLY | ATTACH_VIRTUAL_DISK_FLAG_PERMANENT_LIFETIME,
            0,
            Some(&attach_params),
            None,
        )
        .ok()
        .map_err(|e| {
            log::error!("[ISO] AttachVirtualDisk failed: {:?}", e);
            let _ = CloseHandle(handle);
            e
        })?;

        log::info!("[ISO] Successfully mounted: {:?}", path);
        let _ = CloseHandle(handle);
        Ok(())
    }
}

/// Detects ISO images that are already mounted as virtual CD/DVD drives.
/// Returns a map of drive root (e.g. "D:\\") to the backing ISO file path.
/// Intended to run on a background thread because opening volume handles can block.
pub fn detect_pre_mounted_isos() -> HashMap<String, PathBuf> {
    let mut result = HashMap::new();

    for (drive, _) in super::drives::get_all_drives_fast() {
        if super::system_info::detect_drive_type(&drive) != super::system_info::DriveType::Cdrom {
            continue;
        }

        if let Some(iso_path) = backing_iso_for_drive(&drive) {
            log::info!(
                "[ISO-DETECT] Found pre-mounted ISO: {} -> {}",
                drive,
                iso_path.display()
            );
            result.insert(drive, iso_path);
        }
    }

    result
}

fn backing_iso_for_drive(drive: &str) -> Option<PathBuf> {
    let device_path = format!("\\\\.\\{}", drive.trim_end_matches(['\\', '/']));
    let device_path_wide: Vec<u16> = device_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let handle = CreateFileW(
            PCWSTR(device_path_wide.as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
        .ok()
        .and_then(OwnedHandle::new)?;

        let mut size_used = std::mem::size_of::<STORAGE_DEPENDENCY_INFO>() as u32;
        let mut buffer = aligned_dependency_buffer(size_used);
        let mut info = buffer.as_mut_ptr().cast::<STORAGE_DEPENDENCY_INFO>();
        (*info).Version = STORAGE_DEPENDENCY_INFO_VERSION_2;

        let mut status = GetStorageDependencyInformation(
            handle.as_raw(),
            GET_STORAGE_DEPENDENCY_FLAG_HOST_VOLUMES,
            (buffer.len() * std::mem::size_of::<usize>()) as u32,
            info,
            Some(&mut size_used),
        );

        if status == ERROR_INSUFFICIENT_BUFFER {
            buffer = aligned_dependency_buffer(size_used);
            info = buffer.as_mut_ptr().cast::<STORAGE_DEPENDENCY_INFO>();
            (*info).Version = STORAGE_DEPENDENCY_INFO_VERSION_2;
            status = GetStorageDependencyInformation(
                handle.as_raw(),
                GET_STORAGE_DEPENDENCY_FLAG_HOST_VOLUMES,
                (buffer.len() * std::mem::size_of::<usize>()) as u32,
                info,
                Some(&mut size_used),
            );
        }

        if status != ERROR_SUCCESS {
            log::debug!(
                "[ISO-DETECT] No virtual storage dependency for {}: {}",
                drive,
                status.0
            );
            return None;
        }

        let entries = (*info).Anonymous.Version2Entries.as_ptr();
        for index in 0..(*info).NumberEntries as usize {
            let entry = &*entries.add(index);
            if entry.VirtualStorageType.DeviceId != VIRTUAL_STORAGE_TYPE_DEVICE_ISO
                || entry.HostVolumeName.is_null()
                || entry.DependentVolumeRelativePath.is_null()
            {
                continue;
            }

            let host_volume = entry.HostVolumeName.to_string().ok()?;
            let relative_path = entry.DependentVolumeRelativePath.to_string().ok()?;
            return dependency_path(&host_volume, &relative_path);
        }
    }

    None
}

fn aligned_dependency_buffer(size_bytes: u32) -> Vec<usize> {
    let word_size = std::mem::size_of::<usize>();
    vec![0; (size_bytes as usize).div_ceil(word_size)]
}

fn dependency_path(host_volume: &str, relative_path: &str) -> Option<PathBuf> {
    if host_volume.is_empty() || relative_path.is_empty() {
        return None;
    }

    Some(PathBuf::from(host_volume).join(relative_path.trim_start_matches(['\\', '/'])))
}

/// Detaches a previously mounted ISO file.
/// SAFETY: Interacts with Windows Virtual Disk API.
pub fn detach_iso(path: &Path) -> Result<()> {
    unsafe {
        let path_wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let storage_type = VIRTUAL_STORAGE_TYPE {
            DeviceId: VIRTUAL_STORAGE_TYPE_DEVICE_ISO,
            VendorId: VIRTUAL_STORAGE_TYPE_VENDOR_MICROSOFT,
        };

        let mut handle = HANDLE::default();

        let open_params = OPEN_VIRTUAL_DISK_PARAMETERS {
            Version: OPEN_VIRTUAL_DISK_VERSION_1,
            ..Default::default()
        };

        log::debug!("[ISO] Opening virtual disk for detach (V1): {:?}", path);
        OpenVirtualDisk(
            &storage_type,
            PCWSTR(path_wide.as_ptr()),
            VIRTUAL_DISK_ACCESS_DETACH,
            OPEN_VIRTUAL_DISK_FLAG_NONE,
            Some(&open_params),
            &mut handle,
        )
        .ok()
        .map_err(|e| {
            log::error!("[ISO] OpenVirtualDisk for detach failed: {:?}", e);
            e
        })?;

        log::debug!("[ISO] Detaching virtual disk handle: {:?}", handle);
        DetachVirtualDisk(handle, DETACH_VIRTUAL_DISK_FLAG_NONE, 0)
            .ok()
            .map_err(|e| {
                log::error!("[ISO] DetachVirtualDisk failed: {:?}", e);
                let _ = CloseHandle(handle);
                e
            })?;

        log::info!("[ISO] Successfully detached: {:?}", path);
        let _ = CloseHandle(handle);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::dependency_path;
    use std::path::PathBuf;

    #[test]
    fn dependency_path_preserves_unicode_and_joins_the_volume_root() {
        assert_eq!(
            dependency_path("\\\\?\\Volume{1234}\\", "\\Música\\Vídeo.iso"),
            Some(PathBuf::from("\\\\?\\Volume{1234}\\Música\\Vídeo.iso"))
        );
    }

    #[test]
    fn dependency_path_rejects_missing_components() {
        assert_eq!(dependency_path("", "image.iso"), None);
        assert_eq!(dependency_path("C:\\", ""), None);
    }
}
