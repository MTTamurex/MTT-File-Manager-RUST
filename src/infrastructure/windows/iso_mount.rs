use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Storage::Vhd::*;

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
