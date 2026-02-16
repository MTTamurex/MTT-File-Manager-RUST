//! Windows drive and volume information functions
//! Follows .cursorrules: single responsibility, < 300 lines

use std::path::Path;
use windows::{core::*, Win32::Storage::FileSystem::*, Win32::UI::Shell::*};

/// Volume information structure.
pub struct VolumeInfo {
    pub file_system: String,
    pub total_space: u64,
    pub free_space: u64,
}

/// Gets the label (name) of a Windows volume.
///
/// Uses Shell Display Name (supports virtual drives like Cryptomator).
/// Fallback to GetVolumeInformationW if Shell fails.
pub fn get_volume_label(drive_path: &str) -> String {
    unsafe {
        let path_wide: Vec<u16> = drive_path
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        // First: try Shell Display Name (supports Cryptomator, etc)
        let mut shfi: SHFILEINFOW = std::mem::zeroed();
        let result = SHGetFileInfoW(
            PCWSTR(path_wide.as_ptr()),
            FILE_ATTRIBUTE_DIRECTORY,
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_DISPLAYNAME,
        );

        if result != 0 {
            let display_name = String::from_utf16_lossy(&shfi.szDisplayName)
                .trim_end_matches('\0')
                .to_string();

            // Shell returns "Label (X:)" - extract just the label
            if let Some(paren_pos) = display_name.rfind(" (") {
                let label = display_name[..paren_pos].trim();
                if !label.is_empty() {
                    return label.to_string();
                }
            } else if !display_name.is_empty() {
                return display_name;
            }
        }

        // Fallback: GetVolumeInformationW (real volume label)
        let mut volume_name_buffer = vec![0u16; 256];
        let vol_result = GetVolumeInformationW(
            PCWSTR(path_wide.as_ptr()),
            Some(&mut volume_name_buffer),
            None,
            None,
            None,
            None,
        );

        if vol_result.is_ok() {
            let volume_name = String::from_utf16_lossy(&volume_name_buffer)
                .trim_end_matches('\0')
                .to_string();

            if !volume_name.is_empty() {
                return volume_name;
            }
        }

        "Disco Local".to_string()
    }
}

/// Returns a bitmask of currently available logical drives.
/// Bit 0 = A:, bit 1 = B:, bit 2 = C:, etc.
/// This is extremely fast (no disk I/O) — reads from kernel cache.
pub fn get_logical_drives_bitmask() -> u32 {
    unsafe { GetLogicalDrives() }
}

/// Enumerates all drives with their labels.
pub fn get_all_drives() -> Vec<(String, String)> {
    unsafe {
        // First, get the required length
        let len = GetLogicalDriveStringsW(None);
        if len == 0 {
            return Vec::new();
        }

        let mut buffer = vec![0u16; len as usize];
        let actual_len = GetLogicalDriveStringsW(Some(&mut buffer));

        if actual_len == 0 {
            return Vec::new();
        }

        String::from_utf16_lossy(&buffer[..actual_len as usize])
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(|path| {
                let label = get_volume_label(path);
                let drive_letter = path.trim_end_matches('\\');
                (path.to_string(), format!("{} ({})", label, drive_letter))
            })
            .collect()
    }
}

/// Gets volume information (file system, total/free space).
pub fn get_volume_info(drive_path: &str) -> VolumeInfo {
    unsafe {
        let path_wide: Vec<u16> = drive_path
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let mut sectors_per_cluster = 0u32;
        let mut bytes_per_sector = 0u32;
        let mut free_clusters = 0u32;
        let mut total_clusters = 0u32;

        let result = GetDiskFreeSpaceW(
            PCWSTR(path_wide.as_ptr()),
            Some(&mut sectors_per_cluster),
            Some(&mut bytes_per_sector),
            Some(&mut free_clusters),
            Some(&mut total_clusters),
        );

        let mut file_system = "NTFS".to_string(); // Fallback seguro
        let mut file_system_buffer = vec![0u16; 256];
        let mut volume_serial = 0u32;
        let mut max_component_len = 0u32;
        let mut file_system_flags = 0u32;

        // Use windows-rs wrapper with slices; avoids wrong buffer
        if GetVolumeInformationW(
            PCWSTR(path_wide.as_ptr()),
            None,                          // volume name (not needed)
            Some(&mut volume_serial),      // optional serial
            Some(&mut max_component_len),  // max component length
            Some(&mut file_system_flags),  // flags
            Some(&mut file_system_buffer), // file system name
        )
        .is_ok()
        {
            let fs_str = String::from_utf16_lossy(&file_system_buffer)
                .trim_end_matches('\0')
                .to_string();

            if !fs_str.is_empty() {
                file_system = fs_str;
            }
        }

        if result.is_ok() && sectors_per_cluster > 0 && bytes_per_sector > 0 {
            let bytes_per_cluster = sectors_per_cluster as u64 * bytes_per_sector as u64;
            let total_space = total_clusters as u64 * bytes_per_cluster;
            let free_space = free_clusters as u64 * bytes_per_cluster;

            VolumeInfo {
                file_system,
                total_space,
                free_space,
            }
        } else {
            VolumeInfo {
                file_system,
                total_space: 0,
                free_space: 0,
            }
        }
    }
}

fn extract_drive_letter(path: &Path) -> Option<char> {
    let s = path.to_string_lossy();
    if s.len() >= 2 && s.as_bytes()[1] == b':' {
        s.chars().next().map(|c| c.to_ascii_uppercase())
    } else {
        None
    }
}

/// Returns the filesystem name for a local path drive (e.g. "NTFS", "exFAT").
pub fn get_file_system_for_path(path: &Path) -> Option<String> {
    let drive_letter = extract_drive_letter(path)?;
    let drive_root = format!("{}:\\", drive_letter);
    let info = get_volume_info(&drive_root);
    if info.file_system.is_empty() {
        None
    } else {
        Some(info.file_system)
    }
}

/// USN-capable filesystems support reliable journal-backed change tracking.
pub fn is_usn_filesystem(file_system: &str) -> bool {
    file_system.eq_ignore_ascii_case("NTFS") || file_system.eq_ignore_ascii_case("ReFS")
}

/// Returns whether the path is on an USN-capable filesystem.
pub fn path_is_usn_filesystem(path: &Path) -> Option<bool> {
    get_file_system_for_path(path).map(|fs| is_usn_filesystem(&fs))
}
