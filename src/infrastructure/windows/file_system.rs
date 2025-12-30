//! Windows file system operations
//! Follows .cursorrules: single responsibility, < 300 lines

use std::path::Path;
use crate::infrastructure::windows_api::{
    Win32::Storage::FileSystem::*,
    core::*,
};

/// Gets file attributes for a path.
pub fn get_file_attributes(path: &Path) -> u32 {
    unsafe {
        let path_wide: Vec<u16> = path.to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        
        GetFileAttributesW(PCWSTR(path_wide.as_ptr()))
    }
}

/// Checks if a path is a directory.
pub fn is_directory(path: &Path) -> bool {
    let attrs = get_file_attributes(path);
    attrs != INVALID_FILE_ATTRIBUTES && (attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0
}

/// Checks if a path is a file.
pub fn is_file(path: &Path) -> bool {
    let attrs = get_file_attributes(path);
    attrs != INVALID_FILE_ATTRIBUTES && (attrs & FILE_ATTRIBUTE_DIRECTORY.0) == 0
}
