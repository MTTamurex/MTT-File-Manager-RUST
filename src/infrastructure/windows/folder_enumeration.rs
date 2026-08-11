//! Single-pass subfolder enumeration using Win32 APIs
//!
//! Enumerates the immediate subdirectories of a folder in one
//! `FindFirstFileExW` pass, returning each folder's name and raw
//! `dwFileAttributes`. Attributes come straight from `WIN32_FIND_DATAW`,
//! so no per-entry `metadata()` syscalls are needed by callers.

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES};
use windows::Win32::Storage::FileSystem::{
    FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW,
    FILE_ATTRIBUTE_DIRECTORY, FIND_FIRST_EX_LARGE_FETCH, WIN32_FIND_DATAW,
};

/// A subdirectory entry: name plus raw file attributes from the find data.
#[derive(Debug, Clone)]
pub struct SubfolderAttrs {
    pub name: OsString,
    pub attributes: u32,
}

/// Lists the immediate subdirectories of `path` in a single enumeration pass.
///
/// Uses `FindFirstFileExW` with `FindExInfoBasic` (skips 8.3 short names) and
/// `FIND_FIRST_EX_LARGE_FETCH` (reads the directory table in larger chunks).
/// Returns `None` on I/O error (permission denied, path not found, etc.).
pub fn enumerate_subfolders_attrs(path: &Path) -> Option<Vec<SubfolderAttrs>> {
    let wide_path = search_pattern(path);

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
            Err(error) if error.code() == ERROR_FILE_NOT_FOUND.to_hresult() => {
                return Some(Vec::new());
            }
            Err(_) => return None,
        };

        let completed = loop {
            let attrs = find_data.dwFileAttributes;
            if (attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0 {
                if let Some(name) = extract_name(&find_data.cFileName) {
                    if name != "." && name != ".." {
                        folders.push(SubfolderAttrs {
                            name,
                            attributes: attrs,
                        });
                    }
                }
            }

            if let Err(error) = FindNextFileW(handle, &mut find_data) {
                break error.code() == ERROR_NO_MORE_FILES.to_hresult();
            }
        };

        let _ = FindClose(handle);
        if !completed {
            return None;
        }
    }

    Some(folders)
}

/// Extract the file name from a NUL-terminated wide char array.
fn extract_name(wide_name: &[u16]) -> Option<OsString> {
    let len = wide_name.iter().position(|&c| c == 0)?;
    if len == 0 {
        return None;
    }
    Some(OsString::from_wide(&wide_name[0..len]))
}

fn search_pattern(path: &Path) -> Vec<u16> {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut path_wide: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    for unit in &mut path_wide {
        if *unit == b'/' as u16 {
            *unit = b'\\' as u16;
        }
    }

    const VERBATIM: &[u16] = &[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    const UNC: &[u16] = &[b'\\' as u16, b'\\' as u16];
    const VERBATIM_UNC: &[u16] = &[
        b'\\' as u16,
        b'\\' as u16,
        b'?' as u16,
        b'\\' as u16,
        b'U' as u16,
        b'N' as u16,
        b'C' as u16,
        b'\\' as u16,
    ];

    let mut pattern = if path_wide.starts_with(VERBATIM) {
        path_wide
    } else if path_wide.starts_with(UNC) {
        VERBATIM_UNC
            .iter()
            .copied()
            .chain(path_wide.into_iter().skip(2))
            .collect()
    } else {
        VERBATIM.iter().copied().chain(path_wide).collect()
    };

    if !pattern.ends_with(&[b'\\' as u16]) {
        pattern.push(b'\\' as u16);
    }
    pattern.push(b'*' as u16);
    pattern.push(0);
    pattern
}
