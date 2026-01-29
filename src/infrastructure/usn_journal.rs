use std::collections::HashSet;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsnReason(pub u32);

impl UsnReason {
    pub const DATA_OVERWRITE: u32 = 0x00000001;
    pub const DATA_EXTEND: u32 = 0x00000002;
    pub const DATA_TRUNCATION: u32 = 0x00000004;
    pub const NAMED_DATA_OVERWRITE: u32 = 0x00000010;
    pub const NAMED_DATA_EXTEND: u32 = 0x00000020;
    pub const NAMED_DATA_TRUNCATION: u32 = 0x00000040;
    pub const FILE_CREATE: u32 = 0x00000100;
    pub const FILE_DELETE: u32 = 0x00000200;
    pub const EA_CHANGE: u32 = 0x00000400;
    pub const SECURITY_CHANGE: u32 = 0x00000800;
    pub const RENAME_OLD_NAME: u32 = 0x00001000;
    pub const RENAME_NEW_NAME: u32 = 0x00002000;
    pub const INDEXABLE_CHANGE: u32 = 0x00004000;
    pub const BASIC_INFO_CHANGE: u32 = 0x00008000;
    pub const HARD_LINK_CHANGE: u32 = 0x00010000;
    pub const COMPRESSION_CHANGE: u32 = 0x00020000;
    pub const ENCRYPTION_CHANGE: u32 = 0x00040000;
    pub const OBJECT_ID_CHANGE: u32 = 0x00080000;
    pub const REPARSE_POINT_CHANGE: u32 = 0x00100000;
    pub const STREAM_CHANGE: u32 = 0x00200000;
    pub const CLOSE: u32 = 0x80000000;

    pub fn is_create(&self) -> bool {
        (self.0 & Self::FILE_CREATE) != 0
    }

    pub fn is_delete(&self) -> bool {
        (self.0 & Self::FILE_DELETE) != 0
    }

    pub fn is_rename(&self) -> bool {
        (self.0 & (Self::RENAME_OLD_NAME | Self::RENAME_NEW_NAME)) != 0
    }

    pub fn is_modify(&self) -> bool {
        (self.0 & (Self::DATA_OVERWRITE | Self::DATA_EXTEND | Self::DATA_TRUNCATION)) != 0
    }

    pub fn is_close(&self) -> bool {
        (self.0 & Self::CLOSE) != 0
    }
}

#[derive(Debug, Clone)]
pub struct UsnRecord {
    pub usn: i64,
    pub file_reference_number: u64,
    pub parent_reference_number: u64,
    pub reason: UsnReason,
    pub file_name: String,
    pub file_attributes: u32,
}

impl UsnRecord {
    pub fn is_directory(&self) -> bool {
        (self.file_attributes & 0x10) != 0
    }
}

pub struct UsnJournal {
    handle: HANDLE,
    journal_id: u64,
    first_usn: i64,
    next_usn: i64,
}

#[repr(C)]
struct UsnJournalData {
    usn_journal_id: u64,
    first_usn: i64,
    next_usn: i64,
    lowest_valid_usn: i64,
    max_usn: i64,
    maximum_size: u64,
    allocation_delta: u64,
}

#[repr(C)]
struct ReadUsnJournalData {
    start_usn: i64,
    reason_mask: u32,
    return_only_on_close: u32,
    timeout: u64,
    bytes_to_wait_for: u64,
    usn_journal_id: u64,
}

#[repr(C)]
struct UsnRecordV2 {
    record_length: u32,
    major_version: u16,
    minor_version: u16,
    file_reference_number: u64,
    parent_file_reference_number: u64,
    usn: i64,
    time_stamp: i64,
    reason: u32,
    source_info: u32,
    security_id: u32,
    file_attributes: u32,
    file_name_length: u16,
    file_name_offset: u16,
}

impl UsnJournal {
    pub fn open(drive_letter: char) -> Result<Self, String> {
        let volume_path = format!("\\\\.\\{}:", drive_letter.to_ascii_uppercase());
        let wide_path: Vec<u16> = volume_path.encode_utf16().chain(std::iter::once(0)).collect();

        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide_path.as_ptr()),
                0x80000000,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                None,
            )
        };

        let handle = match handle {
            Ok(h) if h != INVALID_HANDLE_VALUE => h,
            _ => return Err(format!("Failed to open volume {}", drive_letter)),
        };

        let mut journal_data = UsnJournalData {
            usn_journal_id: 0,
            first_usn: 0,
            next_usn: 0,
            lowest_valid_usn: 0,
            max_usn: 0,
            maximum_size: 0,
            allocation_delta: 0,
        };
        let mut bytes_returned: u32 = 0;

        let result = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_QUERY_USN_JOURNAL,
                None,
                0,
                Some(&mut journal_data as *mut _ as *mut std::ffi::c_void),
                std::mem::size_of::<UsnJournalData>() as u32,
                Some(&mut bytes_returned),
                None,
            )
        };

        if result.is_err() {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(format!(
                "USN Journal not available on drive {}",
                drive_letter
            ));
        }

        Ok(Self {
            handle,
            journal_id: journal_data.usn_journal_id,
            first_usn: journal_data.first_usn,
            next_usn: journal_data.next_usn,
        })
    }

    pub fn current_usn(&self) -> i64 {
        self.next_usn
    }

    pub fn first_usn(&self) -> i64 {
        self.first_usn
    }

    pub fn read_changes(&self, start_usn: i64) -> Result<(Vec<UsnRecord>, i64), String> {
        let mut records = Vec::new();
        let mut current_usn = start_usn;
        let mut buffer = vec![0u8; 65536];

        loop {
            let read_data = ReadUsnJournalData {
                start_usn: current_usn,
                reason_mask: 0xFFFFFFFF,
                return_only_on_close: 0,
                timeout: 0,
                bytes_to_wait_for: 0,
                usn_journal_id: self.journal_id,
            };

            let mut bytes_returned: u32 = 0;

            let result = unsafe {
                DeviceIoControl(
                    self.handle,
                    FSCTL_READ_USN_JOURNAL,
                    Some(&read_data as *const _ as *const std::ffi::c_void),
                    std::mem::size_of::<ReadUsnJournalData>() as u32,
                    Some(buffer.as_mut_ptr() as *mut std::ffi::c_void),
                    buffer.len() as u32,
                    Some(&mut bytes_returned),
                    None,
                )
            };

            if result.is_err() || bytes_returned < 8 {
                break;
            }

            let next_usn = i64::from_le_bytes(buffer[0..8].try_into().unwrap());

            if next_usn == current_usn {
                break;
            }

            let mut offset = 8usize;
            while offset < bytes_returned as usize {
                let record_ptr = unsafe { buffer.as_ptr().add(offset) as *const UsnRecordV2 };
                let record = unsafe { &*record_ptr };

                if record.record_length == 0 {
                    break;
                }

                let name_start = offset + record.file_name_offset as usize;
                let name_len = (record.file_name_length / 2) as usize;

                if name_start + name_len * 2 <= bytes_returned as usize {
                    let name_ptr = unsafe { buffer.as_ptr().add(name_start) as *const u16 };
                    let name_slice = unsafe { std::slice::from_raw_parts(name_ptr, name_len) };
                    let file_name = OsString::from_wide(name_slice).to_string_lossy().into_owned();

                    records.push(UsnRecord {
                        usn: record.usn,
                        file_reference_number: record.file_reference_number,
                        parent_reference_number: record.parent_file_reference_number,
                        reason: UsnReason(record.reason),
                        file_name,
                        file_attributes: record.file_attributes,
                    });
                }

                offset += record.record_length as usize;
            }

            current_usn = next_usn;
        }

        Ok((records, current_usn))
    }

    pub fn filter_by_directories(
        &self,
        records: Vec<UsnRecord>,
        monitored_dirs: &HashSet<u64>,
    ) -> Vec<UsnRecord> {
        records
            .into_iter()
            .filter(|r| monitored_dirs.contains(&r.parent_reference_number))
            .collect()
    }
}

impl Drop for UsnJournal {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

pub fn get_file_reference_number(path: &std::path::Path) -> Option<u64> {
    use windows::Win32::Storage::FileSystem::GetFileInformationByHandle;
    use windows::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION;

    let wide_path: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    };

    let handle = match handle {
        Ok(h) if h != INVALID_HANDLE_VALUE => h,
        _ => return None,
    };

    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    let result = unsafe { GetFileInformationByHandle(handle, &mut info) };
    unsafe {
        let _ = CloseHandle(handle);
    }

    if result.is_ok() {
        let file_ref = ((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64);
        Some(file_ref)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_journal() {
        let journal = match UsnJournal::open('C') {
            Ok(j) => j,
            Err(_) => return,
        };

        assert!(journal.current_usn() > 0);
    }

    #[test]
    fn test_read_recent_changes() {
        let journal = match UsnJournal::open('C') {
            Ok(j) => j,
            Err(_) => return,
        };

        let start = journal.current_usn() - 1000000;
        let (records, _) = journal.read_changes(start.max(journal.first_usn())).unwrap();

        assert!(!records.is_empty() || journal.current_usn() == journal.first_usn());
    }
}
