//! Single-pass subfolder enumeration using Win32 APIs
//!
//! Enumerates the immediate subdirectories of a folder in one
//! `FindFirstFileExW` pass, returning each folder's name and raw
//! `dwFileAttributes`. Attributes come straight from `WIN32_FIND_DATAW`,
//! so no per-entry `metadata()` syscalls are needed by callers.

use std::path::Path;
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW,
    FILE_ATTRIBUTE_DIRECTORY, FIND_FIRST_EX_LARGE_FETCH, WIN32_FIND_DATAW,
};

/// A subdirectory entry: name plus raw file attributes from the find data.
#[derive(Debug, Clone)]
pub struct SubfolderAttrs {
    pub name: String,
    pub attributes: u32,
}

/// Lists the immediate subdirectories of `path` in a single enumeration pass.
///
/// Uses `FindFirstFileExW` with `FindExInfoBasic` (skips 8.3 short names) and
/// `FIND_FIRST_EX_LARGE_FETCH` (reads the directory table in larger chunks).
/// Returns `None` on I/O error (permission denied, path not found, etc.).
pub fn enumerate_subfolders_attrs(path: &Path) -> Option<Vec<SubfolderAttrs>> {
    let search_path = if path.to_string_lossy().ends_with('\\') {
        format!("{}*", path.display())
    } else {
        format!("{}\\*", path.display())
    };

    let wide_path: Vec<u16> = search_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let mut find_data = WIN32_FIND_DATAW::default();
    let mut folders = Vec::new();

    unsafe {
        let handle = match FindFirstFileExW(
            PCWSTR(wide_path.as_ptr()),
            FindExInfoBasic,
            &mut find_data as *mut _ as *mut std::ffi::c_void,
            FindExSearchNameMatch,
            Some(std::ptr::null_mut()),
            FIND_FIRST_EX_LARGE_FETCH,
        ) {
            Ok(handle) => handle,
            Err(_) => return None,
        };

        loop {
            let attrs = find_data.dwFileAttributes;
            if (attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0 {
                if let Some(name) = extract_name(&find_data.cFileName) {
                    if name != "." && name != ".." {
                        folders.push(SubfolderAttrs { name, attributes: attrs });
                    }
                }
            }

            if FindNextFileW(handle, &mut find_data).is_err() {
                break;
            }
        }

        let _ = FindClose(handle);
    }

    Some(folders)
}

/// Extract the file name from a NUL-terminated wide char array.
fn extract_name(wide_name: &[u16]) -> Option<String> {
    let len = wide_name.iter().position(|&c| c == 0)?;
    if len == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&wide_name[0..len]))
}
