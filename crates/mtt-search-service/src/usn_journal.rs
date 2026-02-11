use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, WIN32_ERROR};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetVolumeInformationW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;

/// ERROR_HANDLE_EOF (38) - returned by FSCTL_READ_USN_JOURNAL when no new records exist.
const ERROR_HANDLE_EOF: WIN32_ERROR = WIN32_ERROR(38);
/// ERROR_JOURNAL_ENTRY_DELETED (1181) - the saved USN is older than the journal's FirstUsn
/// (journal wrapped around). A full re-scan is needed.
const ERROR_JOURNAL_ENTRY_DELETED: WIN32_ERROR = WIN32_ERROR(1181);

use crate::file_index::{FileRecord, VolumeIndex};

const BUFFER_SIZE: usize = 64 * 1024; // 64KB buffer

// IOCTL codes
const FSCTL_QUERY_USN_JOURNAL: u32 = 0x000900F4;
const FSCTL_ENUM_USN_DATA: u32 = 0x000900B3;
const FSCTL_READ_USN_JOURNAL: u32 = 0x000900BB;

// USN reason flags
const USN_REASON_FILE_CREATE: u32 = 0x00000100;
const USN_REASON_FILE_DELETE: u32 = 0x00000200;
const USN_REASON_RENAME_NEW_NAME: u32 = 0x00002000;
const USN_REASON_CLOSE: u32 = 0x80000000;

// File attributes
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

/// Information about a USN Journal.
pub struct UsnJournalInfo {
    pub journal_id: u64,
    pub first_usn: i64,
    pub next_usn: i64,
}

/// Input structure for FSCTL_ENUM_USN_DATA.
#[repr(C)]
struct MftEnumDataV0 {
    starting_file_reference_number: u64,
    low_usn: i64,
    high_usn: i64,
}

/// Input structure for FSCTL_READ_USN_JOURNAL.
#[repr(C)]
struct ReadUsnJournalDataV0 {
    start_usn: i64,
    reason_mask: u32,
    return_only_on_close: u32,
    timeout: u64,
    bytes_to_wait_for: u64,
    usn_journal_id: u64,
}

/// Discover all NTFS volumes on the system.
/// Returns a list of (drive_letter, volume_label) pairs.
pub fn discover_ntfs_volumes() -> Vec<(char, String)> {
    let mut volumes = Vec::new();

    for letter in 'A'..='Z' {
        let root = format!("{}:\\", letter);
        let root_wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();

        let mut fs_name = [0u16; 64];
        let mut vol_name = [0u16; 256];

        let ok = unsafe {
            GetVolumeInformationW(
                PCWSTR(root_wide.as_ptr()),
                Some(&mut vol_name),
                None,
                None,
                None,
                Some(&mut fs_name),
            )
        };

        if ok.is_ok() {
            let fs = String::from_utf16_lossy(&fs_name)
                .trim_end_matches('\0')
                .to_string();

            if fs == "NTFS" || fs == "ReFS" {
                let label = String::from_utf16_lossy(&vol_name)
                    .trim_end_matches('\0')
                    .to_string();
                volumes.push((letter, label));
            }
        }
    }

    volumes
}

/// Open a handle to the NTFS volume for USN Journal operations.
/// Requires admin/SYSTEM privileges.
pub fn open_volume(drive_letter: char) -> Result<HANDLE, String> {
    let volume_path = format!("\\\\.\\{}:", drive_letter);
    let wide: Vec<u16> = volume_path.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            0x80000000, // GENERIC_READ
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
        .map_err(|e| format!("Failed to open volume {}:\\: {}", drive_letter, e))
    }
}

/// Close a volume handle.
pub fn close_volume(handle: HANDLE) {
    unsafe {
        let _ = CloseHandle(handle);
    }
}

/// Query the USN Journal to get journal ID and boundaries.
pub fn query_usn_journal(volume: HANDLE) -> Result<UsnJournalInfo, String> {
    // USN_JOURNAL_DATA_V2 is 80 bytes
    let mut buffer = [0u8; 80];
    let mut bytes_returned: u32 = 0;

    let result = unsafe {
        DeviceIoControl(
            volume,
            FSCTL_QUERY_USN_JOURNAL,
            None,
            0,
            Some(buffer.as_mut_ptr() as *mut _),
            buffer.len() as u32,
            Some(&mut bytes_returned),
            None,
        )
    };

    if result.is_err() {
        return Err("FSCTL_QUERY_USN_JOURNAL failed. Is the USN Journal enabled?".to_string());
    }

    // Parse USN_JOURNAL_DATA_V0/V1/V2:
    // Offset  0: UsnJournalID (u64)
    // Offset  8: FirstUsn (i64)
    // Offset 16: NextUsn (i64)
    let journal_id = u64::from_le_bytes(buffer[0..8].try_into().unwrap());
    let first_usn = i64::from_le_bytes(buffer[8..16].try_into().unwrap());
    let next_usn = i64::from_le_bytes(buffer[16..24].try_into().unwrap());

    Ok(UsnJournalInfo {
        journal_id,
        first_usn,
        next_usn,
    })
}

/// Enumerate all files on the volume using FSCTL_ENUM_USN_DATA.
/// This walks the entire MFT and is the fastest way to index all files.
pub fn enumerate_all_files(
    volume: HANDLE,
    journal_info: &UsnJournalInfo,
    index: &mut VolumeIndex,
) -> Result<(), String> {
    let mut enum_data = MftEnumDataV0 {
        starting_file_reference_number: 0,
        low_usn: 0,
        high_usn: journal_info.next_usn,
    };

    let mut buffer = vec![0u8; BUFFER_SIZE];
    let mut bytes_returned: u32 = 0;
    let mut total_records: usize = 0;

    loop {
        let result = unsafe {
            DeviceIoControl(
                volume,
                FSCTL_ENUM_USN_DATA,
                Some(&enum_data as *const _ as *const _),
                std::mem::size_of::<MftEnumDataV0>() as u32,
                Some(buffer.as_mut_ptr() as *mut _),
                BUFFER_SIZE as u32,
                Some(&mut bytes_returned),
                None,
            )
        };

        if result.is_err() || bytes_returned < 8 {
            break; // No more data
        }

        // First 8 bytes of output = next starting file reference number
        let next_frn = u64::from_le_bytes(buffer[0..8].try_into().unwrap());

        // Parse USN_RECORD_V2 entries starting after the 8-byte header
        parse_usn_records(&buffer[8..bytes_returned as usize], index, &mut total_records, false);

        // Advance to next batch
        enum_data.starting_file_reference_number = next_frn;
    }

    eprintln!(
        "[USN] {}:\\ Enumerated {} file records",
        index.drive_letter, total_records
    );
    Ok(())
}

/// Read USN Journal changes since last_usn.
/// Returns the new last_usn after processing.
pub fn read_usn_changes(
    volume: HANDLE,
    journal_info: &UsnJournalInfo,
    last_usn: i64,
    index: &mut VolumeIndex,
) -> Result<i64, String> {
    let read_data = ReadUsnJournalDataV0 {
        start_usn: last_usn,
        reason_mask: USN_REASON_FILE_CREATE
            | USN_REASON_FILE_DELETE
            | USN_REASON_RENAME_NEW_NAME
            | USN_REASON_CLOSE,
        return_only_on_close: 0,
        timeout: 0,
        bytes_to_wait_for: 0,
        usn_journal_id: journal_info.journal_id,
    };

    let mut buffer = vec![0u8; BUFFER_SIZE];
    let mut bytes_returned: u32 = 0;

    let result = unsafe {
        DeviceIoControl(
            volume,
            FSCTL_READ_USN_JOURNAL,
            Some(&read_data as *const _ as *const _),
            std::mem::size_of::<ReadUsnJournalDataV0>() as u32,
            Some(buffer.as_mut_ptr() as *mut _),
            BUFFER_SIZE as u32,
            Some(&mut bytes_returned),
            None,
        )
    };

    if result.is_err() {
        let err_code = unsafe { GetLastError() };
        // ERROR_HANDLE_EOF: no new journal records — not a real error
        if err_code == ERROR_HANDLE_EOF {
            return Ok(last_usn);
        }
        // ERROR_JOURNAL_ENTRY_DELETED: cached USN is too old, journal wrapped around
        if err_code == ERROR_JOURNAL_ENTRY_DELETED {
            return Err("journal entries expired (USN too old, full re-scan needed)".to_string());
        }
        return Err(format!(
            "FSCTL_READ_USN_JOURNAL failed (Win32 error {})",
            err_code.0
        ));
    }

    if bytes_returned < 8 {
        return Ok(last_usn);
    }

    // First 8 bytes = next USN
    let new_last_usn = i64::from_le_bytes(buffer[0..8].try_into().unwrap());

    // Parse change records
    let mut dummy_count = 0;
    parse_usn_records(
        &buffer[8..bytes_returned as usize],
        index,
        &mut dummy_count,
        true, // apply changes (delete on FILE_DELETE, update on rename)
    );

    Ok(new_last_usn)
}

/// Parse USN_RECORD_V2 entries from a buffer.
/// If `apply_changes` is true, process DELETE/RENAME reasons.
/// Otherwise, just insert all records (initial enumeration).
fn parse_usn_records(
    data: &[u8],
    index: &mut VolumeIndex,
    count: &mut usize,
    apply_changes: bool,
) {
    let mut offset = 0usize;

    while offset + 64 <= data.len() {
        // USN_RECORD_V2 layout:
        //   0: RecordLength (u32)
        //   4: MajorVersion (u16)
        //   6: MinorVersion (u16)
        //   8: FileReferenceNumber (u64)
        //  16: ParentFileReferenceNumber (u64)
        //  24: Usn (i64)
        //  32: TimeStamp (i64, FILETIME)
        //  40: Reason (u32)
        //  44: SourceInfo (u32)
        //  48: SecurityId (u32)
        //  52: FileAttributes (u32)
        //  56: FileNameLength (u16)
        //  58: FileNameOffset (u16)
        //  60: FileName (UTF-16, variable length)

        let record_length =
            u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;

        if record_length == 0 || offset + record_length > data.len() {
            break;
        }

        // Ensure minimum record size
        if record_length < 60 {
            offset += record_length;
            continue;
        }

        let file_ref = u64::from_le_bytes(data[offset + 8..offset + 16].try_into().unwrap());
        let parent_ref = u64::from_le_bytes(data[offset + 16..offset + 24].try_into().unwrap());
        let reason = u32::from_le_bytes(data[offset + 40..offset + 44].try_into().unwrap());
        let file_attributes =
            u32::from_le_bytes(data[offset + 52..offset + 56].try_into().unwrap());
        let file_name_length =
            u16::from_le_bytes(data[offset + 56..offset + 58].try_into().unwrap()) as usize;
        let file_name_offset =
            u16::from_le_bytes(data[offset + 58..offset + 60].try_into().unwrap()) as usize;

        // Extract file name (UTF-16)
        let name_start = offset + file_name_offset;
        let name_end = name_start + file_name_length;

        if name_end <= offset + record_length && name_end <= data.len() {
            let name_slice = &data[name_start..name_end];
            let name_u16: Vec<u16> = name_slice
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            let name = String::from_utf16_lossy(&name_u16);

            // Strip MFT sequence number (upper 16 bits) to get pure FRN
            let frn = file_ref & 0x0000_FFFF_FFFF_FFFF;
            let parent_frn = parent_ref & 0x0000_FFFF_FFFF_FFFF;

            if apply_changes {
                // Incremental update: process reason flags
                if reason & USN_REASON_FILE_DELETE != 0 {
                    index.records.remove(&frn);
                } else {
                    // Create or update
                    let is_dir = (file_attributes & FILE_ATTRIBUTE_DIRECTORY) != 0;
                    let name_lower = name.to_lowercase();
                    index.records.insert(
                        frn,
                        FileRecord {
                            name,
                            name_lower,
                            parent_ref: parent_frn,
                            is_dir,
                            size: 0,
                        },
                    );
                }
            } else {
                // Initial enumeration: just insert
                let is_dir = (file_attributes & FILE_ATTRIBUTE_DIRECTORY) != 0;
                let name_lower = name.to_lowercase();
                index.records.insert(
                    frn,
                    FileRecord {
                        name,
                        name_lower,
                        parent_ref: parent_frn,
                        is_dir,
                        size: 0,
                    },
                );
                *count += 1;
            }
        }

        offset += record_length;
    }
}
