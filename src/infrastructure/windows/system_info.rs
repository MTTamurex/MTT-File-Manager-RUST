//! Windows system information functions
//! Follows .cursorrules: single responsibility, < 300 lines

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows::{
    Win32::Storage::FileSystem::GetDriveTypeW,
    Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
    Win32::System::Threading::GetCurrentProcess,
};

/// Drive type classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveType {
    Unknown,
    Removable, // Floppy, USB
    Fixed,     // Hard disk, SSD
    Remote,    // Network drive (mapeado)
    Cdrom,     // CD/DVD
    RamDisk,   // RAM disk
}

impl DriveType {
    /// Returns the Windows drive type constant (0-6)
    fn from_windows_type(drive_type: u32) -> Self {
        match drive_type {
            0 => DriveType::Unknown,
            1 => DriveType::Unknown,
            2 => DriveType::Removable,
            3 => DriveType::Fixed,
            4 => DriveType::Remote,
            5 => DriveType::Cdrom,
            6 => DriveType::RamDisk,
            _ => DriveType::Unknown,
        }
    }

    /// Returns a user-friendly string representation
    pub fn label(&self) -> String {
        match self {
            DriveType::Unknown => rust_i18n::t!("drive_types.unknown").to_string(),
            DriveType::Removable => rust_i18n::t!("drive_types.removable").to_string(),
            DriveType::Fixed => rust_i18n::t!("drive_types.fixed").to_string(),
            DriveType::Remote => rust_i18n::t!("drive_types.remote").to_string(),
            DriveType::Cdrom => rust_i18n::t!("drive_types.cdrom").to_string(),
            DriveType::RamDisk => rust_i18n::t!("drive_types.ramdisk").to_string(),
        }
    }

    /// Returns an icon character for the drive type
    pub fn icon(&self) -> &str {
        match self {
            DriveType::Unknown => "?",
            DriveType::Removable => "💾",
            DriveType::Fixed => "💽",
            DriveType::Remote => "🔗",
            DriveType::Cdrom => "📀",
            DriveType::RamDisk => "⚡",
        }
    }
}

/// Detects the type of a drive (local, network, removable, etc)
pub fn detect_drive_type(path: &str) -> DriveType {
    // Ensure path ends with backslash for GetDriveTypeW
    let path_str = if !path.ends_with('\\') {
        format!("{}\\", path)
    } else {
        path.to_string()
    };

    let path_wide: Vec<u16> = OsStr::new(&path_str)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let drive_type = GetDriveTypeW(windows::core::PCWSTR(path_wide.as_ptr()));
        DriveType::from_windows_type(drive_type)
    }
}

/// Gets the current process RAM usage (RSS/Working Set).
pub fn get_ram_usage() -> u64 {
    unsafe {
        let mut counters = PROCESS_MEMORY_COUNTERS::default();
        if K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
        .as_bool()
        {
            counters.WorkingSetSize as u64
        } else {
            0
        }
    }
}
