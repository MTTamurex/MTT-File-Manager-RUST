use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::Path;
use std::sync::OnceLock;
use windows::core::{s, w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, NTSTATUS};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

const BUFFER_SIZE: usize = 65536;
const FILE_DIRECTORY_INFORMATION: u32 = 1;

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

type NtQueryDirectoryFileFn = unsafe extern "system" fn(
    file_handle: HANDLE,
    event: HANDLE,
    apc_routine: *mut std::ffi::c_void,
    apc_context: *mut std::ffi::c_void,
    io_status_block: *mut IoStatusBlock,
    file_information: *mut std::ffi::c_void,
    length: u32,
    file_information_class: u32,
    return_single_entry: u8,
    file_name: *const std::ffi::c_void,
    restart_scan: u8,
) -> NTSTATUS;

#[repr(C)]
struct IoStatusBlock {
    status: NTSTATUS,
    information: usize,
}

static NT_QUERY_DIR: OnceLock<Option<NtQueryDirectoryFileFn>> = OnceLock::new();

fn get_nt_query_directory_file() -> Option<NtQueryDirectoryFileFn> {
    *NT_QUERY_DIR.get_or_init(|| unsafe {
        let ntdll = GetModuleHandleW(w!("ntdll.dll")).ok()?;
        let proc = GetProcAddress(ntdll, s!("NtQueryDirectoryFile"))?;
        Some(std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            NtQueryDirectoryFileFn,
        >(proc))
    })
}

pub fn read_directory_fast(dir_path: &Path) -> Option<Vec<DirectoryEntry>> {
    let nt_query = get_nt_query_directory_file()?;

    let dir_wide: Vec<u16> = dir_path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let handle = unsafe {
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
    };

    let mut entries = Vec::with_capacity(1000);
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let mut restart_scan = 1u8;

    loop {
        let mut io_status = IoStatusBlock {
            status: NTSTATUS(0),
            information: 0,
        };

        let status = unsafe {
            nt_query(
                handle,
                HANDLE::default(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut io_status,
                buffer.as_mut_ptr() as *mut _,
                BUFFER_SIZE as u32,
                FILE_DIRECTORY_INFORMATION,
                0,
                std::ptr::null(),
                restart_scan,
            )
        };

        restart_scan = 0;

        if status.0 as u32 == 0x80000006 {
            break;
        }

        if status.0 != 0 {
            break;
        }

        let mut offset = 0usize;
        loop {
            if offset >= io_status.information {
                break;
            }

            let entry_ptr = unsafe { buffer.as_ptr().add(offset) as *const FileDirectoryInfo };
            let entry = unsafe { &*entry_ptr };

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
                    size: entry.end_of_file as u64,
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

    unsafe {
        let _ = CloseHandle(handle);
    }

    Some(entries)
}

pub fn is_available() -> bool {
    get_nt_query_directory_file().is_some()
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
