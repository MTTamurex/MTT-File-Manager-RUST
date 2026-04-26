use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use windows::core::PCWSTR;
use windows::Wdk::Storage::FileSystem::{FileDirectoryInformation, NtQueryDirectoryFile};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::IO_STATUS_BLOCK;

const BUFFER_SIZE: usize = 65536;

/// Wrapper to ensure the I/O buffer has the alignment required by
/// `FileDirectoryInfo` (8 bytes on x86-64).  A plain `Vec<u8>` has
/// alignment=1, which is formally UB when cast to `*const FileDirectoryInfo`.
#[repr(C, align(8))]
struct AlignedBuffer([u8; BUFFER_SIZE]);

#[repr(C)]
struct FileDirectoryInfo {
    next_entry_offset: u32,
    file_index: u32,
    creation_time: i64,
    last_access_time: i64,
    last_write_time: i64,
    change_time: i64,
    end_of_file: i64,
    allocation_size: i64,
    file_attributes: u32,
    file_name_length: u32,
}

#[derive(Debug, Clone)]
pub struct DirectoryEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64,
    pub attributes: u32,
}

pub fn read_directory_fast(dir_path: &Path) -> Option<Vec<DirectoryEntry>> {
    // H-4: RAII wrapper — CloseHandle guaranteed on return AND panic
    struct HandleGuard(HANDLE);
    impl Drop for HandleGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    let dir_wide: Vec<u16> = dir_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let handle = HandleGuard(unsafe {
        CreateFileW(
            PCWSTR(dir_wide.as_ptr()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
        .ok()?
    });

    let mut entries = Vec::with_capacity(1000);
    let mut buffer = AlignedBuffer([0u8; BUFFER_SIZE]);
    let mut restart_scan = true;

    loop {
        let mut io_status = IO_STATUS_BLOCK::default();

        let status = unsafe {
            NtQueryDirectoryFile(
                handle.0,
                None,
                None,
                None,
                &mut io_status,
                buffer.0.as_mut_ptr() as *mut _,
                BUFFER_SIZE as u32,
                FileDirectoryInformation,
                false,
                None,
                restart_scan,
            )
        };

        restart_scan = false;

        if status.0 as u32 == 0x80000006 {
            break;
        }

        if status.0 != 0 {
            break;
        }

        let mut offset = 0usize;
        loop {
            if offset >= io_status.Information {
                break;
            }

            // Bounds check: ensure the fixed-size entry header fits in the buffer
            let entry_end = offset + std::mem::size_of::<FileDirectoryInfo>();
            if entry_end > io_status.Information {
                break;
            }

            let entry_ptr = unsafe { buffer.0.as_ptr().add(offset) as *const FileDirectoryInfo };
            let entry = unsafe { &*entry_ptr };

            // Bounds check: ensure the variable-length filename fits in the buffer
            let name_end = entry_end + entry.file_name_length as usize;
            if name_end > io_status.Information {
                break;
            }

            let name_ptr =
                unsafe { (entry_ptr as *const u8).add(std::mem::size_of::<FileDirectoryInfo>()) }
                    as *const u16;

            let name_len = (entry.file_name_length / 2) as usize;
            let name_slice = unsafe { std::slice::from_raw_parts(name_ptr, name_len) };
            let name = OsString::from_wide(name_slice)
                .to_string_lossy()
                .into_owned();

            if name != "." && name != ".." {
                let is_dir = (entry.file_attributes & 0x10) != 0;
                let modified = if entry.last_write_time > 116444736000000000 {
                    ((entry.last_write_time as u64) - 116444736000000000) / 10_000_000
                } else {
                    0
                };

                entries.push(DirectoryEntry {
                    name,
                    is_dir,
                    size: entry.end_of_file.max(0) as u64, // M-14: guard against negative value from corrupted NTFS metadata
                    modified,
                    attributes: entry.file_attributes,
                });
            }

            if entry.next_entry_offset == 0 {
                break;
            }
            offset += entry.next_entry_offset as usize;
        }
    }

    // handle dropped here — CloseHandle() guaranteed by HandleGuard RAII
    Some(entries)
}

pub fn is_available() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_directory() {
        if !is_available() {
            return;
        }

        let entries = read_directory_fast(Path::new("C:\\Windows\\System32"));
        assert!(entries.is_some());

        let entries = entries.unwrap();
        assert!(!entries.is_empty());

        let has_dll = entries.iter().any(|e| e.name.ends_with(".dll"));
        assert!(has_dll);
    }
}
