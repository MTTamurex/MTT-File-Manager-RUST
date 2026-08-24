use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, WIN32_ERROR};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetDriveTypeW, GetVolumeInformationW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;

/// ERROR_HANDLE_EOF (38) - returned by FSCTL_READ_USN_JOURNAL when no new records exist.
const ERROR_HANDLE_EOF: WIN32_ERROR = WIN32_ERROR(38);
/// ERROR_JOURNAL_ENTRY_DELETED (1181) - the saved USN is older than the journal's FirstUsn
/// (journal wrapped around). A full re-scan is needed.
const ERROR_JOURNAL_ENTRY_DELETED: WIN32_ERROR = WIN32_ERROR(1181);

use crate::file_index::VolumeIndex;

const BUFFER_SIZE: usize = 64 * 1024; // 64KB buffer

fn log_device_io_control_failure(
    operation: &str,
    volume: HANDLE,
    control_code: u32,
    error: WIN32_ERROR,
) {
    eprintln!(
        "[USN] DeviceIoControl({control_code:#x}, {operation}) on volume handle {:?} failed: win32 error {}",
        volume,
        error.0,
    );
}

// IOCTL codes
const FSCTL_QUERY_USN_JOURNAL: u32 = 0x000900F4;
const FSCTL_READ_USN_JOURNAL: u32 = 0x000900BB;

// USN reason flags
const USN_REASON_FILE_CREATE: u32 = 0x00000100;
const USN_REASON_FILE_DELETE: u32 = 0x00000200;
const USN_REASON_RENAME_NEW_NAME: u32 = 0x00002000;
const USN_REASON_DATA_EXTEND: u32 = 0x00000002;
const USN_REASON_DATA_TRUNCATION: u32 = 0x00000004;
const USN_REASON_DATA_OVERWRITE: u32 = 0x00000001;
const USN_REASON_NAMED_DATA_OVERWRITE: u32 = 0x00000010;
const USN_REASON_NAMED_DATA_EXTEND: u32 = 0x00000020;
const USN_REASON_NAMED_DATA_TRUNCATION: u32 = 0x00000040;
const USN_REASON_BASIC_INFO_CHANGE: u32 = 0x00008000;
const USN_REASON_COMPRESSION_CHANGE: u32 = 0x00020000;
const USN_REASON_ENCRYPTION_CHANGE: u32 = 0x00040000;
const USN_REASON_STREAM_CHANGE: u32 = 0x00200000;
const USN_REASON_CLOSE: u32 = 0x80000000;

/// Combined mask for reasons that indicate a file's data size may have changed.
const USN_REASON_SIZE_CHANGED: u32 = USN_REASON_DATA_EXTEND
    | USN_REASON_DATA_TRUNCATION
    | USN_REASON_DATA_OVERWRITE
    | USN_REASON_NAMED_DATA_OVERWRITE
    | USN_REASON_NAMED_DATA_EXTEND
    | USN_REASON_NAMED_DATA_TRUNCATION
    | USN_REASON_BASIC_INFO_CHANGE
    | USN_REASON_COMPRESSION_CHANGE
    | USN_REASON_ENCRYPTION_CHANGE
    | USN_REASON_STREAM_CHANGE;

// File attributes
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

/// Information about a USN Journal.
pub struct UsnJournalInfo {
    pub journal_id: u64,
    pub first_usn: i64,
    pub next_usn: i64,
}

/// Logical volume discovered on the machine.
#[derive(Clone, Debug)]
pub struct DiscoveredVolume {
    pub drive_letter: char,
    pub label: String,
    pub file_system: String,
    pub usn_supported: bool,
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

/// Discover all mounted logical volumes that expose filesystem information.
pub fn discover_volumes() -> Vec<DiscoveredVolume> {
    // Drive type constants (from winbase.h)
    const DRIVE_REMOTE: u32 = 4;
    const DRIVE_CDROM: u32 = 5;

    let mut volumes = Vec::new();

    for letter in 'A'..='Z' {
        let root = format!("{}:\\", letter);
        let root_wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();

        // Quick pre-filter: GetDriveTypeW is fast and cached (< 1ms).
        // Skip network and optical drives to avoid multi-second timeouts
        // from GetVolumeInformationW on unresponsive/disconnected shares.
        let drive_type = unsafe { GetDriveTypeW(PCWSTR(root_wide.as_ptr())) };
        if drive_type == DRIVE_REMOTE || drive_type == DRIVE_CDROM {
            continue;
        }

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

            if fs.is_empty() {
                continue;
            }

            let label = String::from_utf16_lossy(&vol_name)
                .trim_end_matches('\0')
                .to_string();
            let usn_supported = fs.eq_ignore_ascii_case("NTFS") || fs.eq_ignore_ascii_case("ReFS");

            volumes.push(DiscoveredVolume {
                drive_letter: letter,
                label,
                file_system: fs,
                usn_supported,
            });
        }
    }

    volumes
}

/// Open a handle to the NTFS volume for USN Journal operations.
/// Requires admin/SYSTEM privileges.
pub fn open_volume(drive_letter: char) -> Result<HANDLE, String> {
    let volume_path = format!("\\\\.\\{}:", drive_letter);
    let wide: Vec<u16> = volume_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

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
        let err_code = unsafe { GetLastError() };
        log_device_io_control_failure(
            "FSCTL_QUERY_USN_JOURNAL",
            volume,
            FSCTL_QUERY_USN_JOURNAL,
            err_code,
        );
        return Err(format!(
            "FSCTL_QUERY_USN_JOURNAL failed (Win32 error {}). Is the USN Journal enabled?",
            err_code.0
        ));
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

/// Read USN Journal changes since last_usn.
/// Returns the new last_usn after processing.
pub fn read_usn_changes(
    volume: HANDLE,
    journal_info: &UsnJournalInfo,
    last_usn: i64,
    index: &mut VolumeIndex,
) -> Result<i64, String> {
    let buf = read_usn_buffer(volume, journal_info, last_usn)?;
    let Some((buffer, bytes_returned, new_last_usn)) = buf else {
        return Ok(last_usn);
    };

    let mut dummy_count = 0;
    parse_usn_records(
        &buffer[8..bytes_returned as usize],
        index,
        &mut dummy_count,
        true,
    )?;

    Ok(new_last_usn)
}

/// Read raw USN journal buffer without modifying any index.
/// Returns None when there are no new records.
/// Returns Some((buffer, bytes_returned, new_last_usn)) on success.
/// This performs I/O (DeviceIoControl) and should be called WITHOUT holding locks.
pub fn read_usn_buffer(
    volume: HANDLE,
    journal_info: &UsnJournalInfo,
    last_usn: i64,
) -> Result<Option<(Vec<u8>, u32, i64)>, String> {
    let read_data = ReadUsnJournalDataV0 {
        start_usn: last_usn,
        reason_mask: USN_REASON_FILE_CREATE
            | USN_REASON_FILE_DELETE
            | USN_REASON_RENAME_NEW_NAME
            | USN_REASON_DATA_EXTEND
            | USN_REASON_DATA_TRUNCATION
            | USN_REASON_DATA_OVERWRITE
            | USN_REASON_NAMED_DATA_OVERWRITE
            | USN_REASON_NAMED_DATA_EXTEND
            | USN_REASON_NAMED_DATA_TRUNCATION
            | USN_REASON_BASIC_INFO_CHANGE
            | USN_REASON_COMPRESSION_CHANGE
            | USN_REASON_ENCRYPTION_CHANGE
            | USN_REASON_STREAM_CHANGE
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
        if err_code == ERROR_HANDLE_EOF {
            return Ok(None);
        }
        if err_code == ERROR_JOURNAL_ENTRY_DELETED {
            log_device_io_control_failure(
                "FSCTL_READ_USN_JOURNAL",
                volume,
                FSCTL_READ_USN_JOURNAL,
                err_code,
            );
            return Err("journal entries expired (USN too old, full re-scan needed)".to_string());
        }
        log_device_io_control_failure(
            "FSCTL_READ_USN_JOURNAL",
            volume,
            FSCTL_READ_USN_JOURNAL,
            err_code,
        );
        return Err(format!(
            "FSCTL_READ_USN_JOURNAL failed (Win32 error {})",
            err_code.0
        ));
    }

    if bytes_returned < 8 {
        return Ok(None);
    }

    let new_last_usn = i64::from_le_bytes(buffer[0..8].try_into().unwrap());
    Ok(Some((buffer, bytes_returned, new_last_usn)))
}

/// Parse USN_RECORD_V2 entries from a buffer.
/// If `apply_changes` is true, process DELETE/RENAME reasons.
/// Otherwise, just insert all records (initial enumeration).
pub fn parse_usn_records(
    data: &[u8],
    index: &mut VolumeIndex,
    count: &mut usize,
    apply_changes: bool,
) -> Result<(), String> {
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
                let is_dir = (file_attributes & FILE_ATTRIBUTE_DIRECTORY) != 0;
                if reason & USN_REASON_FILE_DELETE != 0 {
                    let existed = index.records.get(&frn).is_some();
                    index.remove_record(frn);
                    if existed {
                        index
                            .dir_modified_at
                            .insert(parent_frn, std::time::Instant::now());
                    }
                } else if reason & USN_REASON_RENAME_NEW_NAME != 0 {
                    // Rename/move: remove from old parent, add to new parent.
                    let is_reparse = (file_attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0;
                    let old_location = index.records.get(&frn).map(|record| {
                        (
                            record.parent_ref,
                            index.names.try_get(record.name_ref()) == Some(name.as_str()),
                        )
                    });
                    if !index.move_record(frn, &name, parent_frn, is_dir, is_reparse) {
                        return Err(format!("name arena full while moving FRN {frn}"));
                    }
                    let location_changed = old_location
                        .as_ref()
                        .map(|(old_parent, name_unchanged)| {
                            *old_parent != parent_frn || !name_unchanged
                        })
                        .unwrap_or(true);
                    if location_changed {
                        let now = std::time::Instant::now();
                        index.dir_modified_at.insert(parent_frn, now);
                        if let Some((old_parent, _)) = old_location {
                            index.dir_modified_at.insert(old_parent, now);
                        }
                    }
                } else {
                    // Create or update (preserves hardlink children entries).
                    let is_reparse = (file_attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0;
                    let structure_changed = index
                        .records
                        .get(&frn)
                        .map(|record| {
                            record.parent_ref != parent_frn
                                || index.names.try_get(record.name_ref()) != Some(name.as_str())
                                || record.is_dir() != is_dir
                                || index.reparse_points.contains(&frn) != is_reparse
                        })
                        .unwrap_or(true);
                    if !index.insert_record(frn, &name, parent_frn, is_dir, is_reparse) {
                        return Err(format!("name arena full while inserting FRN {frn}"));
                    }
                    if reason & USN_REASON_FILE_CREATE != 0 && structure_changed {
                        index
                            .dir_modified_at
                            .insert(parent_frn, std::time::Instant::now());
                    }
                }

                // Refresh both logical and allocated sizes. This is outside
                // the rename branch because USN can combine rename and stream
                // changes in one record.
                if reason & USN_REASON_FILE_DELETE == 0
                    && !is_dir
                    && reason & (USN_REASON_SIZE_CHANGED | USN_REASON_FILE_CREATE) != 0
                {
                    index.pending_size_refresh.insert(frn);
                }
            } else {
                // Initial enumeration: just insert
                let is_dir = (file_attributes & FILE_ATTRIBUTE_DIRECTORY) != 0;
                let is_reparse = (file_attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0;
                if !index.insert_record_untracked(frn, &name, parent_frn, is_dir, is_reparse) {
                    return Err(format!("name arena full while enumerating FRN {frn}"));
                }
                *count += 1;
            }
        }

        offset += record_length;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        parse_usn_records, FILE_ATTRIBUTE_DIRECTORY, USN_REASON_BASIC_INFO_CHANGE,
        USN_REASON_CLOSE, USN_REASON_DATA_EXTEND, USN_REASON_FILE_CREATE, USN_REASON_FILE_DELETE,
        USN_REASON_RENAME_NEW_NAME,
    };
    use crate::file_index::VolumeIndex;

    fn usn_record(frn: u64, parent: u64, reason: u32, attributes: u32, name: &[u16]) -> Vec<u8> {
        let name_bytes = name.len() * 2;
        let mut record = vec![0u8; 60 + name_bytes];
        let record_len = record.len() as u32;
        record[0..4].copy_from_slice(&record_len.to_le_bytes());
        record[4..6].copy_from_slice(&2u16.to_le_bytes());
        record[8..16].copy_from_slice(&frn.to_le_bytes());
        record[16..24].copy_from_slice(&parent.to_le_bytes());
        record[40..44].copy_from_slice(&reason.to_le_bytes());
        record[52..56].copy_from_slice(&attributes.to_le_bytes());
        record[56..58].copy_from_slice(&(name_bytes as u16).to_le_bytes());
        record[58..60].copy_from_slice(&60u16.to_le_bytes());
        for (chunk, code_unit) in record[60..].chunks_exact_mut(2).zip(name) {
            chunk.copy_from_slice(&code_unit.to_le_bytes());
        }
        record
    }

    fn named_record(frn: u64, parent: u64, reason: u32, attributes: u32, name: &str) -> Vec<u8> {
        usn_record(
            frn,
            parent,
            reason,
            attributes,
            &name.encode_utf16().collect::<Vec<_>>(),
        )
    }

    #[test]
    fn repeated_non_structural_events_do_not_dirty_the_index() {
        let mut index = VolumeIndex::empty('C');
        assert!(index.insert_record(20, "file.bin", 10, false, false));
        index.clear_pending();
        index.binary_dirty = false;
        let arena_len = index.names.len();
        let data = named_record(
            20,
            10,
            USN_REASON_CLOSE | USN_REASON_DATA_EXTEND | USN_REASON_BASIC_INFO_CHANGE,
            0,
            "file.bin",
        );

        parse_usn_records(&data, &mut index, &mut 0, true).unwrap();

        assert_eq!(index.names.len(), arena_len);
        assert!(index.pending_additions.is_empty());
        assert!(!index.binary_dirty);
        assert!(index.pending_size_refresh.contains(&20));
        assert!(index.dir_modified_at.is_empty());
    }

    #[test]
    fn repeated_create_does_not_grow_arena_or_structural_tracking() {
        let mut index = VolumeIndex::empty('C');
        let data = named_record(20, 10, USN_REASON_FILE_CREATE, 0, "file.bin");
        parse_usn_records(&data, &mut index, &mut 0, true).unwrap();
        index.clear_pending();
        index.pending_size_refresh.clear();
        index.dir_modified_at.clear();
        index.binary_dirty = false;
        let arena_len = index.names.len();

        parse_usn_records(&data, &mut index, &mut 0, true).unwrap();

        assert_eq!(index.names.len(), arena_len);
        assert!(index.pending_additions.is_empty());
        assert!(!index.binary_dirty);
        assert!(index.dir_modified_at.is_empty());
    }

    #[test]
    fn rename_marks_old_and_new_parents_only_once() {
        let mut index = VolumeIndex::empty('C');
        assert!(index.insert_record(20, "old.bin", 10, false, false));
        index.dir_modified_at.clear();
        let data = named_record(20, 11, USN_REASON_RENAME_NEW_NAME, 0, "new.bin");

        parse_usn_records(&data, &mut index, &mut 0, true).unwrap();
        assert!(index.dir_modified_at.contains_key(&10));
        assert!(index.dir_modified_at.contains_key(&11));
        let arena_len = index.names.len();

        index.clear_pending();
        index.dir_modified_at.clear();
        index.binary_dirty = false;
        parse_usn_records(&data, &mut index, &mut 0, true).unwrap();
        assert_eq!(index.names.len(), arena_len);
        assert!(index.pending_additions.is_empty());
        assert!(!index.binary_dirty);
        assert!(index.dir_modified_at.is_empty());
    }

    #[test]
    fn absent_delete_is_no_op() {
        let mut index = VolumeIndex::empty('C');
        let data = named_record(
            99,
            10,
            USN_REASON_FILE_DELETE,
            FILE_ATTRIBUTE_DIRECTORY,
            "missing",
        );

        parse_usn_records(&data, &mut index, &mut 0, true).unwrap();

        assert!(index.pending_removals.is_empty());
        assert!(!index.binary_dirty);
        assert!(index.dir_modified_at.is_empty());
    }

    #[test]
    fn insertion_failure_is_propagated_without_advancing_index_usn() {
        let mut index = VolumeIndex::empty('C');
        index.last_usn = 42;
        let oversized_utf8_name = vec![0xD800; (u16::MAX as usize) / 2];
        let data = usn_record(20, 10, USN_REASON_FILE_CREATE, 0, &oversized_utf8_name);

        let result = parse_usn_records(&data, &mut index, &mut 0, true);

        assert!(result.is_err());
        assert_eq!(index.last_usn, 42);
        assert!(index.records.get(&20).is_none());
        assert!(index.pending_additions.is_empty());
        assert!(!index.binary_dirty);
    }
}
