//! Windows drive and volume information functions
//! Follows .cursorrules: single responsibility, < 300 lines

use crate::infrastructure::windows::DriveType;
use std::path::Path;
use windows::{
    core::*,
    Win32::{
        Foundation::{
            CloseHandle, ERROR_ACCESS_DENIED, ERROR_CANCELLED, GetLastError, HWND,
            WAIT_OBJECT_0,
        },
        Storage::FileSystem::*,
        System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE},
        UI::{
            Shell::*,
            WindowsAndMessaging::{
                BringWindowToTop, IsIconic, SetForegroundWindow, ShowWindow, SW_HIDE,
                SW_RESTORE,
            },
        },
    },
};

const ELEVATED_VOLUME_RENAME_FLAG: &str = "--set-volume-label";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeLabelRenameOutcome {
    Renamed,
    RenamedElevated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VolumeLabelRenameError {
    Cancelled,
    InvalidDrivePath,
    InvalidLabel,
    AccessDenied,
    OsError(String),
}

impl std::fmt::Display for VolumeLabelRenameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "{}", rust_i18n::t!("operations.rename_drive_cancelled")),
            Self::InvalidDrivePath => write!(f, "{}", rust_i18n::t!("operations.rename_drive_invalid_target")),
            Self::InvalidLabel => write!(f, "{}", rust_i18n::t!("operations.error_invalid_name")),
            Self::AccessDenied => write!(f, "access denied"),
            Self::OsError(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for VolumeLabelRenameError {}

fn drive_root_from_str(path: &str) -> Option<String> {
    let trimmed = path.trim();
    let mut chars = trimmed.chars();
    let letter = chars.next()?;
    let colon = chars.next()?;
    if !letter.is_ascii_alphabetic() || colon != ':' {
        return None;
    }

    let remainder = chars.as_str();
    if !remainder.is_empty() && remainder != "\\" && remainder != "/" {
        return None;
    }

    Some(format!("{}:\\", letter.to_ascii_uppercase()))
}

pub fn normalize_drive_root_path(path: &Path) -> Option<String> {
    drive_root_from_str(&path.to_string_lossy())
}

pub fn is_drive_root_path(path: &Path) -> bool {
    normalize_drive_root_path(path).is_some()
}

pub fn drive_supports_volume_label_rename(drive_type: DriveType) -> bool {
    matches!(drive_type, DriveType::Fixed | DriveType::Removable | DriveType::RamDisk)
}

pub fn is_valid_volume_label(new_label: &str) -> bool {
    // SEC: NTFS allows up to 32 UTF-16 code units; FAT32 up to 11.
    // Cap at 32 (the more permissive limit) for defense-in-depth.
    new_label.encode_utf16().count() <= 32
        && !new_label.contains('\0')
        && !new_label.contains('\\')
        && !new_label.contains('/')
        && !new_label.contains(':')
        && !new_label.contains('*')
        && !new_label.contains('?')
        && !new_label.contains('"')
        && !new_label.contains('<')
        && !new_label.contains('>')
        && !new_label.contains('|')
}

pub fn format_drive_display_name(drive_path: &str, label: &str) -> String {
    let display_label = if label.trim().is_empty() {
        rust_i18n::t!("drive_types.default_label").to_string()
    } else {
        label.to_string()
    };
    let drive_letter = drive_path.trim_end_matches(['\\', '/']);
    format!("{} ({})", display_label, drive_letter)
}

fn quote_windows_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".to_string();
    }

    let needs_quotes = arg.chars().any(|ch| ch.is_whitespace() || ch == '"');
    if !needs_quotes {
        return arg.to_string();
    }

    let mut quoted = String::from("\"");
    let mut backslashes = 0usize;

    for ch in arg.chars() {
        if ch == '\\' {
            backslashes += 1;
            continue;
        }

        if ch == '"' {
            quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
            quoted.push('"');
            backslashes = 0;
            continue;
        }

        if backslashes > 0 {
            quoted.push_str(&"\\".repeat(backslashes));
            backslashes = 0;
        }

        quoted.push(ch);
    }

    if backslashes > 0 {
        quoted.push_str(&"\\".repeat(backslashes * 2));
    }

    quoted.push('"');
    quoted
}

pub fn restore_window_foreground(hwnd: HWND) {
    if hwnd.is_invalid() {
        return;
    }

    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
    }
}

unsafe fn set_volume_label_raw(
    drive_root: &str,
    new_label: &str,
) -> std::result::Result<(), VolumeLabelRenameError> {
    let drive_wide: Vec<u16> = drive_root.encode_utf16().chain(std::iter::once(0)).collect();
    let label_wide: Vec<u16> = new_label.encode_utf16().chain(std::iter::once(0)).collect();
    let label_ptr = if new_label.is_empty() {
        PCWSTR::null()
    } else {
        PCWSTR(label_wide.as_ptr())
    };

    match SetVolumeLabelW(PCWSTR(drive_wide.as_ptr()), label_ptr) {
        Ok(_) => Ok(()),
        Err(err) => {
            let win32 = GetLastError();
            if win32 == ERROR_ACCESS_DENIED {
                Err(VolumeLabelRenameError::AccessDenied)
            } else {
                Err(VolumeLabelRenameError::OsError(err.to_string()))
            }
        }
    }
}

fn launch_elevated_volume_rename_helper(
    drive_root: &str,
    new_label: &str,
    hwnd: HWND,
) -> std::result::Result<(), VolumeLabelRenameError> {
    let exe = std::env::current_exe()
        .map_err(|err| VolumeLabelRenameError::OsError(err.to_string()))?;
    let exe_wide: Vec<u16> = exe
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let verb_wide: Vec<u16> = "runas".encode_utf16().chain(std::iter::once(0)).collect();
    let params = format!(
        "{} {} {}",
        ELEVATED_VOLUME_RENAME_FLAG,
        quote_windows_arg(drive_root),
        quote_windows_arg(new_label)
    );
    let params_wide: Vec<u16> = params.encode_utf16().chain(std::iter::once(0)).collect();

    let mut exec_info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI,
        hwnd,
        lpVerb: PCWSTR(verb_wide.as_ptr()),
        lpFile: PCWSTR(exe_wide.as_ptr()),
        lpParameters: PCWSTR(params_wide.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };

    unsafe {
        if ShellExecuteExW(&mut exec_info).is_err() {
            let win32 = GetLastError();
            if win32 == ERROR_CANCELLED {
                return Err(VolumeLabelRenameError::Cancelled);
            }

            return Err(VolumeLabelRenameError::OsError(
                windows::core::Error::from_win32().to_string(),
            ));
        }

        let process = exec_info.hProcess;
        if process.is_invalid() {
            return Err(VolumeLabelRenameError::OsError(
                "Missing elevated helper handle".to_string(),
            ));
        }

        let wait = WaitForSingleObject(process, INFINITE);
        if wait != WAIT_OBJECT_0 {
            let _ = CloseHandle(process);
            return Err(VolumeLabelRenameError::OsError(
                windows::core::Error::from_win32().to_string(),
            ));
        }

        let mut exit_code = 1u32;
        if GetExitCodeProcess(process, &mut exit_code).is_err() {
            let _ = CloseHandle(process);
            return Err(VolumeLabelRenameError::OsError(
                windows::core::Error::from_win32().to_string(),
            ));
        }

        let _ = CloseHandle(process);
        restore_window_foreground(hwnd);

        if exit_code == 0 {
            Ok(())
        } else {
            Err(VolumeLabelRenameError::OsError(
                rust_i18n::t!("operations.rename_drive_helper_failed", code = exit_code).to_string(),
            ))
        }
    }
}

pub fn rename_volume_label(
    drive_path: &Path,
    new_label: &str,
    hwnd: HWND,
) -> std::result::Result<VolumeLabelRenameOutcome, VolumeLabelRenameError> {
    let Some(drive_root) = normalize_drive_root_path(drive_path) else {
        return Err(VolumeLabelRenameError::InvalidDrivePath);
    };
    if !is_valid_volume_label(new_label) {
        return Err(VolumeLabelRenameError::InvalidLabel);
    }

    unsafe {
        match set_volume_label_raw(&drive_root, new_label) {
            Ok(()) => Ok(VolumeLabelRenameOutcome::Renamed),
            Err(VolumeLabelRenameError::AccessDenied) => {
                launch_elevated_volume_rename_helper(&drive_root, new_label, hwnd)?;
                Ok(VolumeLabelRenameOutcome::RenamedElevated)
            }
            Err(err) => Err(err),
        }
    }
}

pub fn run_elevated_volume_rename_helper(drive_path: &Path, new_label: &str) -> i32 {
    let Some(drive_root) = normalize_drive_root_path(drive_path) else {
        return 2;
    };
    if !is_valid_volume_label(new_label) {
        return 3;
    }

    unsafe {
        match set_volume_label_raw(&drive_root, new_label) {
            Ok(()) => 0,
            Err(VolumeLabelRenameError::AccessDenied) => 5,
            Err(_) => 1,
        }
    }
}

pub fn get_volume_label_raw(drive_path: &str) -> Option<String> {
    let drive_root = drive_root_from_str(drive_path)?;
    unsafe {
        let path_wide: Vec<u16> = drive_root
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
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
            Some(
                String::from_utf16_lossy(&volume_name_buffer)
                    .trim_end_matches('\0')
                    .to_string(),
            )
        } else {
            None
        }
    }
}

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
        if let Some(volume_name) = get_volume_label_raw(drive_path) {
            if !volume_name.is_empty() {
                return volume_name;
            }
        }

        rust_i18n::t!("drive_types.default_label").to_string()
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
                (path.to_string(), format_drive_display_name(path, &label))
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

        // Use GetDiskFreeSpaceExW for correct 64-bit values on volumes >16TB.
        // GetDiskFreeSpaceW returns 32-bit cluster counts that overflow with
        // 4K sectors on large NTFS volumes.
        let mut free_bytes_available: u64 = 0;
        let mut total_bytes: u64 = 0;
        let mut total_free_bytes: u64 = 0;

        let result = GetDiskFreeSpaceExW(
            PCWSTR(path_wide.as_ptr()),
            Some(&mut free_bytes_available),
            Some(&mut total_bytes),
            Some(&mut total_free_bytes),
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

        if result.is_ok() {
            VolumeInfo {
                file_system,
                total_space: total_bytes,
                free_space: total_free_bytes,
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

pub fn extract_drive_letter(path: &Path) -> Option<char> {
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

/// Checks if a filesystem is known to NOT deliver `ReadDirectoryChangesW`
/// notifications for changes made by other processes.
///
/// Only native FAT-family drivers have this limitation.
/// FUSE-based drivers (Cryptomator/WinFsp, VeraCrypt, etc.) implement
/// `ReadDirectoryChangesW` correctly in their minifilter/driver layer.
pub fn lacks_cross_process_notifications(file_system: &str) -> bool {
    let fs = file_system.trim().to_uppercase();
    matches!(fs.as_str(), "EXFAT" | "FAT32" | "FAT" | "FAT16" | "FAT12")
}

/// Returns whether the path is on a filesystem that lacks cross-process
/// `ReadDirectoryChangesW` notifications (exFAT/FAT32/FAT).
pub fn path_lacks_cross_process_notifications(path: &Path) -> bool {
    get_file_system_for_path(path)
        .map(|fs| lacks_cross_process_notifications(&fs))
        .unwrap_or(false)
}
