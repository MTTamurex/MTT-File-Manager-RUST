//! Windows can register fonts outside the standard font directories (for
//! example, installation using a shortcut). Read those registrations too.

use std::path::PathBuf;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumValueW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
};

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: this handle is owned and was opened successfully below.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

pub(super) fn load_registered_fonts(database: &mut fontdb::Database) {
    let mut loaded: std::collections::HashSet<PathBuf> = database
        .faces()
        .filter_map(|face| match &face.source {
            fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => Some(path.clone()),
            _ => None,
        })
        .collect();
    let path: Vec<_> = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Fonts"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let mut handle = HKEY::default();
        // SAFETY: path is NUL terminated and handle is a writable out parameter.
        if unsafe { RegOpenKeyExW(root, PCWSTR(path.as_ptr()), Some(0), KEY_READ, &mut handle) }
            .is_err()
        {
            continue;
        }
        let key = Key(handle);
        let mut index = 0;
        loop {
            // Registry value names are limited to 16,383 UTF-16 code units.
            let mut name = vec![0u16; 16_384];
            let mut len = name.len() as u32;
            // SAFETY: buffers and lengths match; unused out parameters are null.
            let status = unsafe {
                RegEnumValueW(
                    key.0,
                    index,
                    Some(PWSTR(name.as_mut_ptr())),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if status == ERROR_NO_MORE_ITEMS {
                break;
            }
            if status != ERROR_SUCCESS {
                break;
            }
            if let Some(path) = value_path(&key, &name) {
                // Relative registrations refer to standard directories, which
                // fontdb already scans. Only extra absolute paths are needed.
                if path.is_absolute() && loaded.insert(path.clone()) {
                    let _ = database.load_font_file(path);
                }
            }
            index += 1;
        }
    }
}

fn value_path(key: &Key, name: &[u16]) -> Option<PathBuf> {
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ;
    let mut size = 0;
    // SAFETY: name is a NUL-terminated enumeration buffer; size is writable.
    unsafe {
        RegGetValueW(
            key.0,
            PCWSTR::null(),
            PCWSTR(name.as_ptr()),
            flags,
            None,
            None,
            Some(&mut size),
        )
        .ok()
        .ok()?;
    }
    if size == 0 || size > 65_536 {
        return None;
    }
    let mut data = vec![0u16; (size as usize).div_ceil(2)];
    // RegGetValueW expands environment variables unless NOEXPAND is specified.
    // SAFETY: the allocated UTF-16 buffer is at least size bytes long.
    unsafe {
        RegGetValueW(
            key.0,
            PCWSTR::null(),
            PCWSTR(name.as_ptr()),
            flags,
            None,
            Some(data.as_mut_ptr().cast()),
            Some(&mut size),
        )
        .ok()
        .ok()?;
    }
    let end = data.iter().position(|&c| c == 0).unwrap_or(data.len());
    let value = String::from_utf16(&data[..end]).ok()?;
    (!value.is_empty()).then(|| PathBuf::from(value))
}
