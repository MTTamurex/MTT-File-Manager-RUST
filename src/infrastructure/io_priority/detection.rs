use std::path::Path;
use std::sync::OnceLock;

use rustc_hash::FxHashMap;

/// Cache of disk type detection results (drive letter -> is_ssd).
static DISK_TYPE_CACHE: OnceLock<std::sync::Mutex<FxHashMap<char, bool>>> = OnceLock::new();

fn get_disk_cache() -> &'static std::sync::Mutex<FxHashMap<char, bool>> {
    DISK_TYPE_CACHE.get_or_init(|| std::sync::Mutex::new(FxHashMap::default()))
}

/// Detects if a drive is a virtual Cryptomator drive.
fn is_virtual_drive(drive_letter: char) -> bool {
    crate::infrastructure::virtual_drive_config::detect_virtual_drive(drive_letter).is_some()
}

fn extract_drive_letter(path: &Path) -> Option<char> {
    path.to_str()
        .and_then(|s| {
            if s.len() >= 2 && s.chars().nth(1) == Some(':') {
                s.chars().next()
            } else {
                None
            }
        })
        .map(|c| c.to_ascii_uppercase())
}

/// Checks whether a path belongs to a virtual drive.
pub(super) fn is_virtual_drive_path(path: &Path) -> bool {
    if path
        .to_str()
        .map(|s| s.to_lowercase().starts_with(r"\\cryptomator-vault\"))
        .unwrap_or(false)
    {
        return true;
    }

    let Some(drive_letter) = extract_drive_letter(path) else {
        return false;
    };

    if crate::infrastructure::virtual_drive_config::get_drive_override(drive_letter).is_some() {
        return true;
    }

    is_virtual_drive(drive_letter)
}

/// Detect if a path is on an SSD (no seek penalty) or HDD (has seek penalty).
pub(super) fn is_ssd(path: &Path) -> bool {
    let Some(drive_letter) = extract_drive_letter(path) else {
        return true; // Assume SSD for network paths, etc.
    };

    if let Ok(cache) = get_disk_cache().lock() {
        if let Some(&is_ssd) = cache.get(&drive_letter) {
            return is_ssd;
        }
    }

    let result = determine_disk_type(drive_letter);

    if let Ok(mut cache) = get_disk_cache().lock() {
        cache.insert(drive_letter, result);
    }

    result
}

/// Non-blocking cache lookup for disk type.
/// Returns:
/// - Some(is_ssd) when cached
/// - None when drive is unknown (caller may choose a safe fallback without probing)
pub(super) fn try_is_ssd_cached(path: &Path) -> Option<bool> {
    let drive_letter = extract_drive_letter(path)?;
    let cache = get_disk_cache().lock().ok()?;
    cache.get(&drive_letter).copied()
}

fn determine_disk_type(drive_letter: char) -> bool {
    // Check user configuration first (manual overrides).
    if let Some(override_type) =
        crate::infrastructure::virtual_drive_config::get_drive_override(drive_letter)
    {
        return matches!(
            override_type,
            crate::infrastructure::virtual_drive_config::DiskTypeOverride::SSD
        );
    }

    if is_virtual_drive(drive_letter) {
        // Default to SSD for unconfigured virtual drives (safe default).
        return true;
    }

    query_disk_seek_penalty(drive_letter)
}

/// Invalidate cache for a specific drive (useful after configuration changes).
pub(super) fn invalidate_drive_cache(drive_letter: char) {
    if let Ok(mut cache) = get_disk_cache().lock() {
        cache.remove(&drive_letter.to_ascii_uppercase());
    }
}

/// Query Windows for whether a disk has seek penalty (HDD) or not (SSD).
fn query_disk_seek_penalty(drive_letter: char) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Ioctl::IOCTL_STORAGE_QUERY_PROPERTY;
    use windows::Win32::System::IO::DeviceIoControl;

    let device_path = format!("\\\\.\\{}:", drive_letter);
    let wide_path: Vec<u16> = device_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let handle = CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        );

        let handle = match handle {
            Ok(h) if h != INVALID_HANDLE_VALUE => h,
            _ => return true,
        };

        const STORAGE_DEVICE_SEEK_PENALTY_PROPERTY: u32 = 7;
        const PROPERTY_STANDARD_QUERY: u32 = 0;

        #[repr(C)]
        struct StoragePropertyQuery {
            property_id: u32,
            query_type: u32,
            additional_parameters: [u8; 1],
        }

        #[repr(C)]
        struct DeviceSeekPenaltyDescriptor {
            version: u32,
            size: u32,
            incurs_seek_penalty: u8,
        }

        let query = StoragePropertyQuery {
            property_id: STORAGE_DEVICE_SEEK_PENALTY_PROPERTY,
            query_type: PROPERTY_STANDARD_QUERY,
            additional_parameters: [0],
        };

        let mut result = DeviceSeekPenaltyDescriptor {
            version: 0,
            size: 0,
            incurs_seek_penalty: 1,
        };

        let mut bytes_returned: u32 = 0;

        let success = DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&query as *const _ as *const std::ffi::c_void),
            std::mem::size_of::<StoragePropertyQuery>() as u32,
            Some(&mut result as *mut _ as *mut std::ffi::c_void),
            std::mem::size_of::<DeviceSeekPenaltyDescriptor>() as u32,
            Some(&mut bytes_returned),
            None,
        );

        let _ = CloseHandle(handle);

        if success.is_ok() && bytes_returned > 0 {
            let is_ssd = result.incurs_seek_penalty == 0;
            log::debug!(
                "[DISK-DETECT] Drive {}: DeviceIoControl succeeded, is_ssd={}",
                drive_letter,
                is_ssd
            );
            is_ssd
        } else {
            // Query failed - default to HDD for safer performance behavior on unknown devices.
            false
        }
    }
}
