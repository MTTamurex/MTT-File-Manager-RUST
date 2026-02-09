use std::path::Path;

use super::DirectoryEntries;

pub(super) fn read_directory_with_timeout(
    path: &Path,
    timeout_ms: u64,
) -> super::IoTimeoutResult<DirectoryEntries> {
    if !super::is_onedrive_path(path) {
        return match read_directory_internal(path) {
            Ok(entries) => super::IoTimeoutResult::Ok(entries),
            Err(_) => super::IoTimeoutResult::Err(std::io::ErrorKind::Other),
        };
    }

    let effective_timeout = if super::is_app_minimized() {
        timeout_ms / 2
    } else {
        timeout_ms
    };
    super::timeout_ops::run_onedrive_timeout_operation(
        path,
        effective_timeout,
        5,
        "read_directory()",
        move |path_buf| match read_directory_internal(&path_buf) {
            Ok(entries) => super::IoTimeoutResult::Ok(entries),
            Err(_) => super::IoTimeoutResult::Err(std::io::ErrorKind::Other),
        },
    )
}

pub(super) fn onedrive_read_directory(path: &Path) -> super::IoTimeoutResult<DirectoryEntries> {
    read_directory_with_timeout(path, super::ONEDRIVE_DIR_ENUM_TIMEOUT_MS)
}

fn read_directory_internal(path: &Path) -> Result<DirectoryEntries, std::io::Error> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        FindClose, FindExInfoBasic, FindExInfoStandard, FindExSearchNameMatch, FindFirstFileExW,
        FindFirstFileW, FindNextFileW, FIND_FIRST_EX_FLAGS, FIND_FIRST_EX_LARGE_FETCH,
        WIN32_FIND_DATAW,
    };

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
    let mut entries = Vec::new();

    unsafe {
        // Fast path: Explorer-like enumeration flags to reduce OneDrive overhead.
        // Fallback to standard Win32 call for compatibility.
        let handle = match FindFirstFileExW(
            PCWSTR(wide_path.as_ptr()),
            FindExInfoBasic,
            &mut find_data as *mut _ as *mut std::ffi::c_void,
            FindExSearchNameMatch,
            Some(std::ptr::null_mut()),
            FIND_FIRST_EX_LARGE_FETCH,
        ) {
            Ok(h) => h,
            Err(_) => match FindFirstFileExW(
                PCWSTR(wide_path.as_ptr()),
                FindExInfoStandard,
                &mut find_data as *mut _ as *mut std::ffi::c_void,
                FindExSearchNameMatch,
                Some(std::ptr::null_mut()),
                FIND_FIRST_EX_FLAGS(0),
            ) {
                Ok(h) => h,
                Err(_) => FindFirstFileW(PCWSTR(wide_path.as_ptr()), &mut find_data)?,
            },
        };

        loop {
            let len = find_data
                .cFileName
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(find_data.cFileName.len());
            let filename = OsString::from_wide(&find_data.cFileName[0..len])
                .to_string_lossy()
                .into_owned();

            if filename != "." && filename != ".." {
                let attrs = find_data.dwFileAttributes;
                let is_dir = (attrs & 0x10) != 0;

                let size = if is_dir {
                    0
                } else {
                    ((find_data.nFileSizeHigh as u64) << 32) | (find_data.nFileSizeLow as u64)
                };

                let ft = find_data.ftLastWriteTime;
                let windows_ticks = ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
                let modified = if windows_ticks > 116444736000000000 {
                    (windows_ticks - 116444736000000000) / 10_000_000
                } else {
                    0
                };

                entries.push((filename, attrs, size, modified));
            }

            if FindNextFileW(handle, &mut find_data).is_err() {
                break;
            }
        }

        let _ = FindClose(handle);
    }

    Ok(entries)
}
