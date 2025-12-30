//! Windows drive and volume information functions
//! Follows .cursorrules: single responsibility, < 300 lines

use windows::{
    Win32::Storage::FileSystem::*,
    Win32::UI::Shell::*,
    core::*,
};

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

/// Enumerates all drives with their labels.
pub fn get_all_drives() -> Vec<(String, String)> {
    unsafe {
        let mut buffer = vec![0u16; 256];
        let len = GetLogicalDriveStringsW(Some(&mut buffer));
        
        if len == 0 {
            return Vec::new();
        }
        
        String::from_utf16_lossy(&buffer[..len as usize])
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
        
        let mut file_system = "Desconhecido".to_string();
        let mut max_component_len = 0u32;
        let mut file_system_flags = 0u32;
        
        let mut file_system_buffer_u32 = vec![0u32; 256];
        
        if GetVolumeInformationW(
            PCWSTR(path_wide.as_ptr()),
            None,
            Some(file_system_buffer_u32.as_mut_ptr()),
            Some(&mut max_component_len),
            Some(&mut file_system_flags),
            None,
        ).is_ok() {
            let file_system_u16: Vec<u16> = file_system_buffer_u32
                .iter()
                .take_while(|&&c| c != 0)
                .map(|&c| c as u16)
                .collect();
            file_system = String::from_utf16_lossy(&file_system_u16);
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
