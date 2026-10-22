//! Single-directory Win32 enumeration with bounded progress/cancellation intervals.
use super::{
    build_child_wide, build_search_pattern, is_dot_or_dotdot, is_junction_or_symlink,
    FolderScanResult,
};
use std::sync::atomic::{AtomicBool, Ordering};
use windows::core::PCWSTR;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW,
    FILE_ATTRIBUTE_DIRECTORY, FIND_FIRST_EX_LARGE_FETCH, WIN32_FIND_DATAW,
};

struct FindHandle(HANDLE);

impl Drop for FindHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = FindClose(self.0);
        }
    }
}

/// Return local totals and collect child paths without UTF conversions.
pub(super) fn scan_dir_wide_local(
    dir_wide: &[u16],
    sub_dirs: &mut Vec<Vec<u16>>,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> FolderScanResult {
    let search = build_search_pattern(dir_wide);
    let mut find_data = WIN32_FIND_DATAW::default();
    let mut totals = FolderScanResult::default();
    let mut entries_since_progress = 0u16;

    unsafe {
        let handle = FindFirstFileExW(
            PCWSTR(search.as_ptr()),
            FindExInfoBasic,
            &mut find_data as *mut _ as *mut std::ffi::c_void,
            FindExSearchNameMatch,
            Some(std::ptr::null_mut()),
            FIND_FIRST_EX_LARGE_FETCH,
        );

        let handle = match handle {
            Ok(h) => h,
            Err(error) => {
                totals.inaccessible_dir_count = 1;
                totals.last_error_code = error.code().0;
                return totals;
            }
        };
        // Progress callbacks can unwind; always release the enumeration handle.
        let handle = FindHandle(handle);

        loop {
            let attrs = find_data.dwFileAttributes;
            let is_dir = (attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0;

            if is_dir {
                if !is_dot_or_dotdot(&find_data.cFileName)
                    && !is_junction_or_symlink(attrs, find_data.dwReserved0)
                {
                    totals.folder_count = totals.folder_count.saturating_add(1);
                    sub_dirs.push(build_child_wide(dir_wide, &find_data.cFileName));
                }
            } else {
                totals.file_count = totals.file_count.saturating_add(1);
                totals.total_size = totals.total_size.saturating_add(
                    ((find_data.nFileSizeHigh as u64) << 32) | (find_data.nFileSizeLow as u64),
                );
            }

            // Large flat directories must also publish progress and observe cancellation.
            entries_since_progress += 1;
            if entries_since_progress == 256 {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                progress(totals.total_size);
                entries_since_progress = 0;
            }

            if FindNextFileW(handle.0, &mut find_data).is_err() {
                break;
            }
        }
    }

    totals
}
