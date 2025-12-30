//! Windows shell operations
//! Follows .cursorrules: single responsibility, < 300 lines

use std::path::Path;
use windows::{
    Win32::UI::Shell::*,
    Win32::UI::WindowsAndMessaging::*,
    core::*,
};

/// Opens a file with its default application using ShellExecuteW.
pub fn open_with_shell(path: &Path) {
    unsafe {
        let path_str = path.to_string_lossy().to_string();
        let path_wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();
        
        let _ = ShellExecuteW(
            None,
            PCWSTR(std::ptr::null()),
            PCWSTR(path_wide.as_ptr()),
            PCWSTR(std::ptr::null()),
            PCWSTR(std::ptr::null()),
            SW_SHOW,
        );
    }
}
