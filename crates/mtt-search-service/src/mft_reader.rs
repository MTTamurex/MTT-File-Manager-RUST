//! Raw MFT reader for extracting file sizes from NTFS volumes.
//!
//! After the USN-based enumeration builds the FRN→FileRecord index,
//! this module performs a second sequential pass over the MFT to extract
//! the `$DATA` attribute size for each file, populating `FileRecord.size`.
//!
//! The MFT is read sequentially in large chunks, making this I/O-efficient
//! even on HDDs (one contiguous read instead of per-file stat calls).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HANDLE, GetLastError};
use windows::Win32::Storage::FileSystem::{ReadFile, SetFilePointerEx, FILE_BEGIN};
use windows::Win32::System::IO::DeviceIoControl;

use crate::file_index::VolumeIndex;

// ── FSCTL codes ─────────────────────────────────────────────────────────────

/// FSCTL_GET_NTFS_VOLUME_DATA — returns NTFS_VOLUME_DATA_BUFFER with MFT geometry.
const FSCTL_GET_NTFS_VOLUME_DATA: u32 = 0x00090064;

/// FSCTL_GET_NTFS_FILE_RECORD — read a single MFT record by FRN.
const FSCTL_GET_NTFS_FILE_RECORD: u32 = 0x00090068;

/// Size of the NTFS_FILE_RECORD_OUTPUT_BUFFER header before the actual record data.
const OUTPUT_HEADER: usize = 12; // FileReferenceNumber(8) + FileRecordLength(4)

// ── MFT record constants ────────────────────────────────────────────────────

/// MFT record signature: "FILE" in little-endian.
const FILE_SIGNATURE: [u8; 4] = [b'F', b'I', b'L', b'E'];

/// $DATA attribute type identifier.
const ATTR_TYPE_DATA: u32 = 0x80;

/// $STANDARD_INFORMATION attribute type identifier.
const ATTR_TYPE_STANDARD_INFORMATION: u32 = 0x10;

/// $FILE_NAME attribute type identifier — contains parent FRN + name.
const ATTR_TYPE_FILE_NAME: u32 = 0x30;

/// $ATTRIBUTE_LIST type identifier — points to attributes stored in external records.
const ATTR_TYPE_ATTRIBUTE_LIST: u32 = 0x20;

/// $REPARSE_POINT attribute type identifier — contains the reparse tag.
const ATTR_TYPE_REPARSE_POINT: u32 = 0xC0;

/// End-of-attributes marker.
const ATTR_TYPE_END: u32 = 0xFFFF_FFFF;

/// Returns `true` if the reparse tag identifies a cloud/OneDrive folder.
/// These are the ONLY reparse-point directories that Explorer counts when
/// computing folder size via right-click → Properties.  All other reparse
/// directories (junctions, symlinks, mount points, etc.) are skipped.
#[inline]
fn is_cloud_reparse_tag(tag: u32) -> bool {
    // IO_REPARSE_TAG_CLOUD variants: 0x9000_X01A where X = 0..F
    (tag & 0xFFFF_0FFF == 0x9000_001A)
    // IO_REPARSE_TAG_ONEDRIVE
    || tag == 0x8000_0021
}

/// MFT record header flag: record is in use.
const MFT_RECORD_IN_USE: u16 = 0x01;

/// MFT record header flag: record is a directory.
const MFT_RECORD_IS_DIRECTORY: u16 = 0x02;

/// Minimum size of an $ATTRIBUTE_LIST entry (without the variable-length name).
const ATTR_LIST_ENTRY_MIN_SIZE: usize = 26; // type(4)+len(2)+name_len(1)+name_off(1)+start_vcn(8)+base_ref(8)+id(2)

// ── NTFS_VOLUME_DATA_BUFFER layout (offsets) ────────────────────────────────
// This struct is 96 bytes (NTFS_VOLUME_DATA_BUFFER).
// We only need a few fields:
//   Offset  0: VolumeSerialNumber (i64)
//   Offset  8: NumberSectors (i64)
//   Offset 16: TotalClusters (i64)
//   Offset 24: FreeClusters (i64)
//   Offset 32: TotalReserved (i64)
//   Offset 40: BytesPerSector (u32)
//   Offset 44: BytesPerCluster (u32)
//   Offset 48: BytesPerFileRecordSegment (u32)
//   Offset 52: ClustersPerFileRecordSegment (u32)
//   Offset 56: MftValidDataLength (i64)
//   Offset 64: MftStartLcn (i64)
//   Offset 72: Mft2StartLcn (i64)
//   Offset 80: MftZoneStart (i64)
//   Offset 88: MftZoneEnd (i64)

/// Geometry of the MFT on an NTFS volume.
struct MftGeometry {
    bytes_per_sector: u32,
    bytes_per_cluster: u32,
    bytes_per_file_record: u32,
    mft_start_lcn: i64,
    mft_valid_data_length: i64,
}

/// Query NTFS volume data to get MFT location and record size.
fn query_mft_geometry(volume: HANDLE) -> Result<MftGeometry, String> {
    let mut buffer = [0u8; 96];
    let mut bytes_returned: u32 = 0;

    let result = unsafe {
        DeviceIoControl(
            volume,
            FSCTL_GET_NTFS_VOLUME_DATA,
            None,
            0,
            Some(buffer.as_mut_ptr() as *mut _),
            buffer.len() as u32,
            Some(&mut bytes_returned),
            None,
        )
    };

    if result.is_err() {
        let err = unsafe { GetLastError() };
        return Err(format!(
            "FSCTL_GET_NTFS_VOLUME_DATA failed (Win32 error {})",
            err.0
        ));
    }

    if bytes_returned < 96 {
        return Err(format!(
            "FSCTL_GET_NTFS_VOLUME_DATA returned {} bytes (expected 96)",
            bytes_returned
        ));
    }

    let bytes_per_sector = u32::from_le_bytes(buffer[40..44].try_into().unwrap());
    let bytes_per_cluster = u32::from_le_bytes(buffer[44..48].try_into().unwrap());
    let bytes_per_file_record = u32::from_le_bytes(buffer[48..52].try_into().unwrap());
    let mft_valid_data_length = i64::from_le_bytes(buffer[56..64].try_into().unwrap());
    let mft_start_lcn = i64::from_le_bytes(buffer[64..72].try_into().unwrap());

    // Sanity checks
    if bytes_per_file_record == 0 || bytes_per_cluster == 0 {
        return Err("Invalid MFT geometry: zero-size records or clusters".to_string());
    }
    if bytes_per_file_record > 16384 {
        return Err(format!(
            "Unexpected MFT record size: {} bytes",
            bytes_per_file_record
        ));
    }

    Ok(MftGeometry {
        bytes_per_sector,
        bytes_per_cluster,
        bytes_per_file_record,
        mft_start_lcn,
        mft_valid_data_length,
    })
}

/// Extract the $DATA size from an attribute at `offset` within `record`.
/// Handles both resident and non-resident attributes. Only considers default
/// (unnamed) $DATA streams — skips Alternate Data Streams.
fn extract_data_size_at(record: &[u8], offset: usize, record_size: usize) -> Option<u64> {
    if offset + 16 > record_size {
        return None;
    }
    let attr_type = u32::from_le_bytes(record[offset..offset + 4].try_into().unwrap());
    if attr_type != ATTR_TYPE_DATA {
        return None;
    }
    // name_length at offset+9: non-zero means Alternate Data Stream.
    let name_length = record[offset + 9];
    if name_length != 0 {
        return None;
    }

    let non_resident_flag = record[offset + 8];
    if non_resident_flag == 0 {
        // Resident attribute: value_length at attr_offset+0x10.
        if offset + 0x14 <= record_size {
            let value_length =
                u32::from_le_bytes(record[offset + 0x10..offset + 0x14].try_into().unwrap());
            return Some(value_length as u64);
        }
    } else {
        // Non-resident attribute: real_size at attr_offset+0x30.
        // The real_size field is consistent across all copies of a
        // non-resident attribute header (base and continuation records),
        // so we read it regardless of lowest_vcn.
        if offset + 0x38 <= record_size {
            let real_size =
                u64::from_le_bytes(record[offset + 0x30..offset + 0x38].try_into().unwrap());
            return Some(real_size);
        }
    }
    None
}

/// Extract all unique parent FRNs from $FILE_NAME attributes in an MFT record.
///
/// NTFS hardlinked files have multiple $FILE_NAME attributes, each pointing
/// to a different parent directory. `FSCTL_ENUM_USN_DATA` only returns ONE
/// name per FRN, so hardlinks are invisible to the USN enumeration.
///
/// This function walks the record's attributes and returns all unique parent
/// FRNs (48-bit) found in $FILE_NAME entries. Returns empty if the MFT
/// header's `link_count` is ≤ 1 (no hardlinks) or the record is invalid.
fn extract_hardlink_parents(record: &[u8], record_size: usize) -> Vec<u64> {
    if record_size < 0x18 || record[0..4] != FILE_SIGNATURE {
        return Vec::new();
    }
    let flags = u16::from_le_bytes(record[0x16..0x18].try_into().unwrap());
    if flags & MFT_RECORD_IN_USE == 0 || flags & MFT_RECORD_IS_DIRECTORY != 0 {
        return Vec::new();
    }
    // link_count at offset 0x12 — number of hard links.
    let link_count = u16::from_le_bytes(record[0x12..0x14].try_into().unwrap());
    if link_count <= 1 {
        return Vec::new();
    }

    let first_attr = u16::from_le_bytes(record[0x14..0x16].try_into().unwrap()) as usize;
    if first_attr >= record_size || first_attr < 0x30 {
        return Vec::new();
    }

    // SEC: Cap pre-allocation to avoid excessive memory from corrupted MFT records
    // claiming a very high link_count. The real count is bounded by record_size.
    let mut parents: Vec<u64> = Vec::with_capacity(link_count.min(64) as usize);
    let mut offset = first_attr;
    while offset + 16 <= record_size {
        let attr_type = u32::from_le_bytes(record[offset..offset + 4].try_into().unwrap());
        if attr_type == ATTR_TYPE_END {
            break;
        }
        let attr_length =
            u32::from_le_bytes(record[offset + 4..offset + 8].try_into().unwrap()) as usize;
        if attr_length == 0 || offset + attr_length > record_size {
            break;
        }

        if attr_type == ATTR_TYPE_FILE_NAME {
            // $FILE_NAME is always resident. Content layout:
            //   0..8:  parent directory FRN (u64, low 48 bits = FRN)
            //   ...    (other fields: timestamps, sizes, name, etc.)
            let non_resident = record[offset + 8];
            if non_resident == 0 && offset + 0x18 <= record_size {
                let value_offset = u16::from_le_bytes(
                    record[offset + 0x14..offset + 0x16].try_into().unwrap(),
                ) as usize;
                let content_start = offset + value_offset;
                if content_start + 8 <= record_size {
                    let parent_frn = u64::from_le_bytes(
                        record[content_start..content_start + 8].try_into().unwrap(),
                    ) & 0x0000_FFFF_FFFF_FFFF;
                    if !parents.contains(&parent_frn) {
                        parents.push(parent_frn);
                    }
                }
            }
        }

        offset += attr_length;
    }

    parents
}

/// Result of parsing an MFT record for the $DATA size.
enum MftSizeResult {
    /// Found the $DATA size directly in the record.
    Found(u64),
    /// $DATA lives in an external MFT record referenced via $ATTRIBUTE_LIST.
    /// Contains a list of FRNs (48-bit) of external records that hold $DATA.
    External(Vec<u64>),
    /// Record is not a valid in-use file, or has no $DATA at all.
    None,
}

/// Parse a single MFT record and extract the default $DATA stream size.
///
/// If the record contains a `$DATA` attribute directly, returns `Found(size)`.
/// If the record references `$DATA` via an `$ATTRIBUTE_LIST` pointing to
/// external MFT records, returns `External(frn_list)` so the caller can
/// fetch those records and retry.
fn parse_mft_record(record: &[u8], record_size: usize) -> MftSizeResult {
    if record.len() < record_size || record_size < 48 {
        return MftSizeResult::None;
    }

    // Check FILE signature.
    if record[0..4] != FILE_SIGNATURE {
        return MftSizeResult::None;
    }

    // Flags at offset 0x16 (22).
    let flags = u16::from_le_bytes(record[0x16..0x18].try_into().unwrap());
    if flags & MFT_RECORD_IN_USE == 0 {
        return MftSizeResult::None;
    }
    if flags & MFT_RECORD_IS_DIRECTORY != 0 {
        return MftSizeResult::None;
    }

    // First attribute offset at offset 0x14 (20).
    let first_attr_offset = u16::from_le_bytes(record[0x14..0x16].try_into().unwrap()) as usize;
    if first_attr_offset >= record_size || first_attr_offset < 0x30 {
        return MftSizeResult::None;
    }

    let mut external_data_frns: Vec<u64> = Vec::new();

    // Walk attribute list.
    let mut offset = first_attr_offset;
    while offset + 16 <= record_size {
        let attr_type = u32::from_le_bytes(record[offset..offset + 4].try_into().unwrap());
        if attr_type == ATTR_TYPE_END {
            break;
        }

        let attr_length =
            u32::from_le_bytes(record[offset + 4..offset + 8].try_into().unwrap()) as usize;
        if attr_length == 0 || offset + attr_length > record_size {
            break;
        }

        // ── Pass 1: Direct $DATA attribute ──
        if attr_type == ATTR_TYPE_DATA {
            if let Some(size) = extract_data_size_at(record, offset, record_size) {
                return MftSizeResult::Found(size);
            }
        }

        // ── Pass 2: $ATTRIBUTE_LIST — scan for $DATA entries in external records ──
        if attr_type == ATTR_TYPE_ATTRIBUTE_LIST {
            let is_non_resident = record[offset + 8];
            if is_non_resident == 0 {
                // Resident $ATTRIBUTE_LIST: value is inline.
                if offset + 0x14 <= record_size {
                    let value_length = u32::from_le_bytes(
                        record[offset + 0x10..offset + 0x14].try_into().unwrap(),
                    ) as usize;
                    let value_offset = u16::from_le_bytes(
                        record[offset + 0x14..offset + 0x16].try_into().unwrap(),
                    ) as usize;
                    let abs_start = offset + value_offset;
                    let abs_end = abs_start.saturating_add(value_length).min(record_size);

                    parse_attribute_list_for_data(
                        &record[abs_start..abs_end],
                        &mut external_data_frns,
                    );
                }
            }
            // Non-resident $ATTRIBUTE_LIST: we cannot follow data runs with
            // FSCTL_GET_NTFS_FILE_RECORD, so skip. The caller's fallback tiers
            // will handle this case.
        }

        offset += attr_length;
    }

    if !external_data_frns.is_empty() {
        MftSizeResult::External(external_data_frns)
    } else {
        MftSizeResult::None
    }
}

/// Scan an $ATTRIBUTE_LIST value for entries of type $DATA that reference
/// external MFT records. Pushes their FRNs (48-bit) into `out`.
fn parse_attribute_list_for_data(list_data: &[u8], out: &mut Vec<u64>) {
    let mut pos = 0usize;
    while pos + ATTR_LIST_ENTRY_MIN_SIZE <= list_data.len() {
        // $ATTRIBUTE_LIST entry layout:
        //   0..4:   attribute type (u32)
        //   4..6:   entry length (u16)
        //   6:      name length (u8)
        //   7:      name offset (u8)
        //   8..16:  starting VCN (u64)
        //  16..24:  base file reference (u64) — low 48 bits = FRN
        //  24..26:  attribute ID (u16)
        let entry_type =
            u32::from_le_bytes(list_data[pos..pos + 4].try_into().unwrap());
        let entry_length =
            u16::from_le_bytes(list_data[pos + 4..pos + 6].try_into().unwrap()) as usize;

        if entry_length < ATTR_LIST_ENTRY_MIN_SIZE || pos + entry_length > list_data.len() {
            break;
        }

        if entry_type == ATTR_TYPE_DATA {
            let name_length = list_data[pos + 6];
            // Only default (unnamed) $DATA — skip ADS entries.
            if name_length == 0 {
                // Only follow the first extent (starting_vcn == 0). Other
                // extents hold data-run continuations whose real_size field
                // is unreliable per NTFS spec.
                let starting_vcn = u64::from_le_bytes(
                    list_data[pos + 8..pos + 16].try_into().unwrap(),
                );
                if starting_vcn == 0 {
                    let base_ref = u64::from_le_bytes(
                        list_data[pos + 16..pos + 24].try_into().unwrap(),
                    ) & 0x0000_FFFF_FFFF_FFFF;
                    out.push(base_ref);
                }
            }
        }

        pos += entry_length;
        // $ATTRIBUTE_LIST entries are NOT 8-byte aligned. The entry_length
        // field already accounts for any padding. Advancing by entry_length
        // alone is correct (matches ntfs3, ntfs-3g behaviour).
    }
}

/// Fetch one MFT record by FRN via `FSCTL_GET_NTFS_FILE_RECORD`.
/// Returns the raw record data if the IOCTL succeeds and the returned FRN matches.
fn fetch_mft_record(
    volume: HANDLE,
    frn: u64,
    record_size: usize,
    output_buffer: &mut Vec<u8>,
) -> Option<usize> {
    let input = (frn as i64).to_le_bytes();
    let mut bytes_returned: u32 = 0;
    let output_size = OUTPUT_HEADER + record_size;
    if output_buffer.len() < output_size {
        output_buffer.resize(output_size, 0);
    }

    let result = unsafe {
        DeviceIoControl(
            volume,
            FSCTL_GET_NTFS_FILE_RECORD,
            Some(input.as_ptr() as *const _),
            input.len() as u32,
            Some(output_buffer.as_mut_ptr() as *mut _),
            output_size as u32,
            Some(&mut bytes_returned),
            None,
        )
    };

    if result.is_err() || (bytes_returned as usize) < OUTPUT_HEADER + 48 {
        return None;
    }

    let actual_frn =
        u64::from_le_bytes(output_buffer[0..8].try_into().unwrap()) & 0x0000_FFFF_FFFF_FFFF;
    if actual_frn != frn {
        return None; // IOCTL returned nearest valid record, not ours.
    }

    let returned_record_length =
        u32::from_le_bytes(output_buffer[8..12].try_into().unwrap()) as usize;
    let available = (bytes_returned as usize).saturating_sub(OUTPUT_HEADER);
    let parse_len = returned_record_length.min(available).min(record_size);
    if parse_len < 48 {
        return None;
    }

    Some(parse_len)
}

/// How a file size was resolved.
enum SizeResolution {
    /// Found $DATA directly in the base MFT record.
    Direct(u64),
    /// Found $DATA in an external record via $ATTRIBUTE_LIST.
    ViaAttrList(u64),
    /// Could not resolve.
    None,
}

/// Try to extract the $DATA size for a single FRN.
/// Handles $ATTRIBUTE_LIST by fetching referenced external records (up to 4).
fn resolve_file_size(
    volume: HANDLE,
    frn: u64,
    record_size: usize,
    output_buffer: &mut Vec<u8>,
) -> SizeResolution {
    let parse_len = match fetch_mft_record(volume, frn, record_size, output_buffer) {
        Some(len) => len,
        None => return SizeResolution::None,
    };
    let record_data = &output_buffer[OUTPUT_HEADER..OUTPUT_HEADER + parse_len];

    match parse_mft_record(record_data, parse_len) {
        MftSizeResult::Found(size) => SizeResolution::Direct(size),
        MftSizeResult::External(ext_frns) => {
            // Follow $ATTRIBUTE_LIST: fetch each external record and look for $DATA.
            let mut ext_buf = vec![0u8; OUTPUT_HEADER + record_size];
            for ext_frn in ext_frns.iter().take(4) {
                if let Some(ext_len) =
                    fetch_mft_record(volume, *ext_frn, record_size, &mut ext_buf)
                {
                    let ext_data = &ext_buf[OUTPUT_HEADER..OUTPUT_HEADER + ext_len];
                    // Walk attributes of the external record looking for $DATA.
                    if ext_data.len() >= 48 && ext_data[0..4] == FILE_SIGNATURE {
                        let first_attr = u16::from_le_bytes(
                            ext_data[0x14..0x16].try_into().unwrap(),
                        ) as usize;
                        if first_attr >= 0x30 && first_attr < ext_len {
                            let mut off = first_attr;
                            while off + 16 <= ext_len {
                                let atype = u32::from_le_bytes(
                                    ext_data[off..off + 4].try_into().unwrap(),
                                );
                                if atype == ATTR_TYPE_END {
                                    break;
                                }
                                let alen = u32::from_le_bytes(
                                    ext_data[off + 4..off + 8].try_into().unwrap(),
                                ) as usize;
                                if alen == 0 || off + alen > ext_len {
                                    break;
                                }
                                if let Some(size) =
                                    extract_data_size_at(ext_data, off, ext_len)
                                {
                                    return SizeResolution::ViaAttrList(size);
                                }
                                off += alen;
                            }
                        }
                    }
                }
            }
            SizeResolution::None
        }
        MftSizeResult::None => SizeResolution::None,
    }
}

/// Fallback path-based size query for files whose MFT record did not yield a
/// usable `$DATA` size. This is slower than raw MFT parsing, so it is only
/// used for misses.
// ── ACL-aware folder size ──────────────────────────────────────────────

/// Check whether the *current thread* token is explicitly **denied** access
/// to enumerate a directory.  Uses `FindFirstFileW` (the same API that
/// Explorer Properties uses) so the result matches what Explorer would skip.
///
/// Returns `true` only when `FindFirstFileW` fails with an access-denied
/// HRESULT.  Any other failure (path not found, sharing violation, etc.)
/// is **not** treated as access-denied.
fn is_directory_access_denied(path: &str) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        FindClose, FindFirstFileW, WIN32_FIND_DATAW,
    };

    let pattern = if path.ends_with('\\') {
        format!("{}*", path)
    } else {
        format!("{}\\*", path)
    };
    let wide: Vec<u16> = pattern.encode_utf16().chain(std::iter::once(0)).collect();
    let mut find_data = WIN32_FIND_DATAW::default();
    match unsafe { FindFirstFileW(PCWSTR(wide.as_ptr()), &mut find_data) } {
        Ok(h) => {
            let _ = unsafe { FindClose(h) };
            false // accessible
        }
        Err(e) => {
            // Only treat ERROR_ACCESS_DENIED (5) as denied.
            e.code().0 as u32 == 0x80070005 // HRESULT for Win32 ERROR_ACCESS_DENIED
        }
    }
}

/// Compute folder size respecting the current thread's security context.
///
/// Uses `VolumeIndex::folder_size_sum` (fast in-memory BFS, no FRN dedup,
/// hardlinks counted per appearance — matching Explorer) as the base total,
/// then **subtracts** only the subtrees of immediate child directories for
/// which `FindFirstFileW` returns explicit `ERROR_ACCESS_DENIED`.
///
/// This is a **fail-open** approach: if path resolution fails or `CreateFileW`
/// fails for any reason other than ACCESS_DENIED, the subtree is kept.  Only
/// an explicit ACL denial (like `C:\Program Files\WindowsApps`) is subtracted.
///
/// Returns `(total_size, file_count, skipped_dirs)`.
pub fn folder_size_for_user(
    index: &VolumeIndex,
    dir_frn: u64,
) -> (u64, u64, u64) {
    // Start with the full in-memory total (no dedup, includes hardlinks).
    let (raw_total, raw_count) = index.folder_size_sum(dir_frn);

    // Diagnostic: compute unique-file total to detect hardlink impact.
    let (uniq_total, uniq_count, dup_hits) = index.folder_size_sum_unique_files(dir_frn);
    if dup_hits > 0 || raw_total != uniq_total {
        eprintln!(
            "[FOLDER-SIZE-DIAG] frn={} raw={:.2}GB({} files) unique={:.2}GB({} files) dup_hits={} delta={:.2}MB",
            dir_frn,
            raw_total as f64 / 1_073_741_824.0,
            raw_count,
            uniq_total as f64 / 1_073_741_824.0,
            uniq_count,
            dup_hits,
            (raw_total as i128 - uniq_total as i128) as f64 / 1_048_576.0,
        );
    }

    let mut denied_size: u64 = 0;
    let mut denied_count: u64 = 0;
    let mut skipped_dirs: u64 = 0;
    let mut dir_cache: HashMap<u64, String> = HashMap::with_capacity(64);

    // ACL-check only the immediate child directories.
    if let Some(child_frns) = index.children.get(&dir_frn) {
        for &child_frn in child_frns {
            if let Some(record) = index.records.get(&child_frn) {
                if !record.is_dir {
                    continue;
                }

                // Resolve path and check for explicit ACCESS_DENIED.
                let denied = crate::path_resolver::resolve_path_cached(
                    child_frn, index, &mut dir_cache,
                )
                .map_or(false, |p| is_directory_access_denied(&p));

                if denied {
                    // Subtract this subtree's contribution.
                    let (sub_size, sub_count) = index.folder_size_sum(child_frn);
                    let name = index.names.get(record.name_ref());
                    eprintln!(
                        "[FOLDER-SIZE-ACL] blocked child frn={} name={:?} size={:.2}GB files={}",
                        child_frn,
                        name,
                        sub_size as f64 / 1_073_741_824.0,
                        sub_count,
                    );
                    denied_size = denied_size.saturating_add(sub_size);
                    denied_count = denied_count.saturating_add(sub_count);
                    skipped_dirs += 1;
                }
            }
        }
    }

    (
        raw_total.saturating_sub(denied_size),
        raw_count.saturating_sub(denied_count),
        skipped_dirs,
    )
}

fn stat_file_size_fallback(
    index: &VolumeIndex,
    frn: u64,
    dir_cache: &mut HashMap<u64, String>,
) -> Option<u64> {
    let path = crate::path_resolver::resolve_path_cached(frn, index, dir_cache)?;
    let path = if path.starts_with(r"\\?\") {
        path
    } else {
        format!(r"\\?\{}", path)
    };

    let metadata = std::fs::metadata(path).ok()?;
    if metadata.is_file() {
        Some(metadata.len())
    } else {
        None
    }
}

/// Open a file directly by its FRN (File Reference Number) and query its size.
/// Uses `OpenFileById` from kernel32 — no path resolution needed. This is
/// the last-resort fallback for files whose parent chain can't be resolved.
fn size_by_file_id(volume: HANDLE, frn: u64) -> Option<u64> {
    use windows::Win32::Foundation::CloseHandle;

    // FILE_ID_DESCRIPTOR layout for FileIdType (8-byte NTFS FRN):
    //   Offset 0:  dwSize (u32) = 24
    //   Offset 4:  Type   (u32) = 0 (FileIdType)
    //   Offset 8:  FileId (i64)
    //   Offset 16: padding (8 bytes — union extends to 16 for FILE_ID_128)
    #[repr(C)]
    struct FileIdDescriptor {
        dw_size: u32,
        id_type: u32,
        file_id: i64,
        _pad: [u8; 8],
    }

    extern "system" {
        fn OpenFileById(
            h_volume_hint: *mut std::ffi::c_void,
            lp_file_id: *const FileIdDescriptor,
            dw_desired_access: u32,
            dw_share_mode: u32,
            lp_security_attributes: *const std::ffi::c_void,
            dw_flags_and_attributes: u32,
        ) -> *mut std::ffi::c_void;

        fn GetFileSizeEx(h_file: *mut std::ffi::c_void, lp_file_size: *mut i64) -> i32;
    }

    let desc = FileIdDescriptor {
        dw_size: 24,
        id_type: 0, // FileIdType
        file_id: frn as i64,
        _pad: [0; 8],
    };

    const FILE_READ_ATTRIBUTES: u32 = 0x0080;
    const FILE_SHARE_ALL: u32 = 0x07; // READ | WRITE | DELETE
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

    let raw_handle = unsafe {
        OpenFileById(
            volume.0,
            &desc,
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_ALL,
            std::ptr::null(),
            FILE_FLAG_BACKUP_SEMANTICS,
        )
    };

    let invalid = -1isize as *mut std::ffi::c_void;
    if raw_handle.is_null() || raw_handle == invalid {
        return None;
    }

    let handle = HANDLE(raw_handle);
    let mut size: i64 = 0;
    let ok = unsafe { GetFileSizeEx(raw_handle, &mut size) };
    let _ = unsafe { CloseHandle(handle) };

    if ok != 0 && size >= 0 {
        Some(size as u64)
    } else {
        None
    }
}

/// Read all file sizes from the MFT and populate `FileRecord.size` in the index.
///
/// Uses `FSCTL_GET_NTFS_FILE_RECORD` per target FRN. When the record's `$DATA`
/// attribute lives in an external record (via `$ATTRIBUTE_LIST`), follows the
/// reference to extract the size. Falls back to `std::fs::metadata` or
/// `OpenFileById` only for records that still can't be resolved.
///
/// Returns the number of file records whose sizes were successfully extracted.
pub fn read_file_sizes<F>(
    volume: HANDLE,
    index: &mut VolumeIndex,
    mut on_progress: F,
) -> Result<usize, String>
where
    F: FnMut(u64, u64),
{
    let geometry = query_mft_geometry(volume)?;

    eprintln!(
        "[MFT-SIZE] {}:\\ MFT geometry: record_size={}, cluster_size={}, mft_start_lcn={}, valid_data={:.1}MB",
        index.drive_letter,
        geometry.bytes_per_file_record,
        geometry.bytes_per_cluster,
        geometry.mft_start_lcn,
        geometry.mft_valid_data_length as f64 / 1_048_576.0
    );

    let record_size = geometry.bytes_per_file_record as usize;
    let total_records = geometry.mft_valid_data_length as u64 / record_size as u64;

    // Build a set of FRNs we care about (non-directory files in our index).
    let target_frns: Vec<u64> = index
        .records
        .iter()
        .filter(|(_, rec)| !rec.is_dir)
        .map(|(&frn, _)| frn)
        .collect();

    if target_frns.is_empty() {
        eprintln!("[MFT-SIZE] {}:\\ No file records to size", index.drive_letter);
        index.sizes_loaded = true;
        on_progress(0, 0);
        return Ok(0);
    }

    eprintln!(
        "[MFT-SIZE] {}:\\ Reading sizes for {} files from {} MFT records...",
        index.drive_letter,
        target_frns.len(),
        total_records
    );

    let start = std::time::Instant::now();

    let mut sized_count: usize = 0;
    let mut mft_direct_count: usize = 0;
    let mut attr_list_count: usize = 0;
    let mut fallback_count: usize = 0;
    let mut file_id_count: usize = 0;
    let mut unresolved_count: usize = 0;
    let mut new_hardlink_edges: usize = 0;
    let mut dir_cache: HashMap<u64, String> = HashMap::with_capacity(4096);
    let mut output_buffer = vec![0u8; OUTPUT_HEADER + record_size];
    let total_targets = target_frns.len() as u64;
    let mut processed_count = 0u64;
    let mut last_reported_count = 0u64;
    let mut last_report_at = Instant::now();

    on_progress(0, total_targets);

    for &frn in &target_frns {
        // Tier 1: MFT parsing (with $ATTRIBUTE_LIST follow).
        let mft_size = resolve_file_size(volume, frn, record_size, &mut output_buffer);

        let size = match mft_size {
            SizeResolution::Direct(s) => {
                mft_direct_count += 1;
                Some(s)
            }
            SizeResolution::ViaAttrList(s) => {
                attr_list_count += 1;
                Some(s)
            }
            SizeResolution::None => {
                // Tier 2: metadata fallback via path resolution.
                if let Some(s) = stat_file_size_fallback(index, frn, &mut dir_cache) {
                    fallback_count += 1;
                    Some(s)
                }
                // Tier 3: OpenFileById fallback.
                else if let Some(s) = size_by_file_id(volume, frn) {
                    file_id_count += 1;
                    Some(s)
                } else {
                    unresolved_count += 1;
                    None
                }
            }
        };

        if let Some(size) = size {
            if let Some(file_record) = index.records.get_mut(&frn) {
                file_record.size = size;
                sized_count += 1;
            }
        }

        // ── Hardlink detection ──────────────────────────────────────────
        // FSCTL_ENUM_USN_DATA returns ONE record per FRN, so files with
        // multiple hard links (e.g. WinSxS) only appear under one parent.
        // The base MFT record is still in output_buffer after resolve_file_size
        // (external records are fetched into a separate buffer).
        // Extract all $FILE_NAME parent FRNs and register missing edges.
        {
            let returned_len =
                u32::from_le_bytes(output_buffer[8..12].try_into().unwrap()) as usize;
            let parse_len = returned_len.min(record_size);
            if parse_len >= 48 {
                let record_data = &output_buffer[OUTPUT_HEADER..OUTPUT_HEADER + parse_len];
                let all_parents = extract_hardlink_parents(record_data, parse_len);
                if all_parents.len() > 1 {
                    let primary_parent = index.records.get(&frn).map(|r| r.parent_ref);
                    for &parent in &all_parents {
                        if Some(parent) != primary_parent {
                            let extras = index.hardlink_parents.entry(frn).or_default();
                            if !extras.contains(&parent) {
                                extras.push(parent);
                                new_hardlink_edges += 1;
                            }
                        }
                    }
                }
            }
        }

        processed_count += 1;
        if processed_count != last_reported_count
            && (processed_count.saturating_sub(last_reported_count) >= 1_024
                || last_report_at.elapsed() >= Duration::from_millis(120))
        {
            on_progress(processed_count, total_targets);
            last_reported_count = processed_count;
            last_report_at = Instant::now();
        }
    }

    if processed_count != last_reported_count {
        on_progress(processed_count, total_targets);
    }

    let elapsed = start.elapsed();
    eprintln!(
        "[MFT-SIZE] {}:\\ Extracted sizes for {} files in {:.2}s (MFT direct: {}, $ATTR_LIST: {}, metadata fb: {}, file-id fb: {}, unresolved: {})",
        index.drive_letter,
        sized_count,
        elapsed.as_secs_f64(),
        mft_direct_count,
        attr_list_count,
        fallback_count,
        file_id_count,
        unresolved_count
    );

    // ── Rebuild children index if MFT-based hardlink detection found new edges ──
    if new_hardlink_edges > 0 {
        eprintln!(
            "[MFT-SIZE] {}:\\ Discovered {} new hardlink parent edges from MFT $FILE_NAME scan, rebuilding children index",
            index.drive_letter,
            new_hardlink_edges
        );
        index.rebuild_children();
    }

    // Diagnostic: compare total record sizes vs BFS-reachable sizes from root.
    // A large gap here means files exist in `records` but aren't reachable via
    // the children reverse index — indicating broken parent chains.
    {
        let total_all_files: u64 = index
            .records
            .values()
            .filter(|r| !r.is_dir)
            .map(|r| r.size)
            .sum();
        let (bfs_total, bfs_count) = index.folder_size_sum(5); // ROOT_FRN = 5
        let file_count_all = index.records.values().filter(|r| !r.is_dir).count() as u64;
        let unreachable_files = file_count_all.saturating_sub(bfs_count);
        let gap_bytes = total_all_files.saturating_sub(bfs_total);
        eprintln!(
            "[MFT-SIZE] {}:\\ Diagnostic: all_records={:.2}GB ({} files), bfs_from_root={:.2}GB ({} files), gap={:.2}GB ({} files unreachable)",
            index.drive_letter,
            total_all_files as f64 / 1_073_741_824.0,
            file_count_all,
            bfs_total as f64 / 1_073_741_824.0,
            bfs_count,
            gap_bytes as f64 / 1_073_741_824.0,
            unreachable_files,
        );

        // If there's a meaningful gap, identify orphan root FRNs —
        // the parent FRNs that are NOT in `records` (broken chain heads).
        if unreachable_files > 0 {
            // BFS-reachable set
            let mut reachable = std::collections::HashSet::with_capacity(bfs_count as usize + 1000);
            reachable.insert(5u64);
            let mut stack = vec![5u64];
            while let Some(frn) = stack.pop() {
                if let Some(child_frns) = index.children.get(&frn) {
                    for &child in child_frns {
                        if reachable.insert(child) {
                            if index.records.get(&child).map_or(false, |r| r.is_dir) {
                                stack.push(child);
                            }
                        }
                    }
                }
            }

            // Count orphan files grouped by their root-cause parent
            // (the ancestor whose parent_ref is not in records).
            let mut orphan_parent_counts: HashMap<u64, (usize, u64)> = HashMap::new();
            for (&frn, record) in &index.records {
                if record.is_dir || reachable.contains(&frn) {
                    continue;
                }
                // Walk parent chain to find the break point.
                let mut ancestor = record.parent_ref;
                for _ in 0..256 {
                    if ancestor == 5 || !index.records.contains_key(&ancestor) {
                        break;
                    }
                    if reachable.contains(&ancestor) {
                        break;
                    }
                    let parent = index.records[&ancestor].parent_ref;
                    if parent == ancestor {
                        break;
                    }
                    ancestor = parent;
                }
                let entry = orphan_parent_counts.entry(ancestor).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += record.size;
            }

            // Sort by size descending and show top entries.
            let mut orphan_roots: Vec<_> = orphan_parent_counts.into_iter().collect();
            orphan_roots.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
            let show = orphan_roots.len().min(10);
            for &(root_frn, (count, size)) in &orphan_roots[..show] {
                let name = index
                    .records
                    .get(&root_frn)
                    .map(|r| index.names.get(r.name_ref()).to_string())
                    .unwrap_or_else(|| format!("(missing FRN {})", root_frn));
                eprintln!(
                    "[MFT-SIZE] {}:\\ Orphan root: FRN={} name={:?} → {} files, {:.2}MB",
                    index.drive_letter,
                    root_frn,
                    name,
                    count,
                    size as f64 / 1_048_576.0,
                );
            }
            if orphan_roots.len() > show {
                eprintln!(
                    "[MFT-SIZE] {}:\\ ... and {} more orphan roots",
                    index.drive_letter,
                    orphan_roots.len() - show
                );
            }
        }
    }

    index.sizes_loaded = true;
    Ok(sized_count)
}

/// Read the size of a single file by FRN using FSCTL_GET_NTFS_FILE_RECORD.
/// Follows $ATTRIBUTE_LIST references when necessary.
/// Used for incremental updates when a file's size changes.
pub fn read_single_file_size(volume: HANDLE, frn: u64, record_size: u32) -> Option<u64> {
    let rs = record_size as usize;
    let mut output_buffer = vec![0u8; OUTPUT_HEADER + rs];
    match resolve_file_size(volume, frn, rs, &mut output_buffer) {
        SizeResolution::Direct(s) | SizeResolution::ViaAttrList(s) => Some(s),
        SizeResolution::None => None,
    }
}

/// Public wrapper to query the MFT record size for a volume.
/// Returns `bytes_per_file_record` on success.
pub fn query_mft_geometry_pub(volume: HANDLE) -> Result<u32, String> {
    query_mft_geometry(volume).map(|g| g.bytes_per_file_record)
}

// ── Bulk MFT reader ─────────────────────────────────────────────────────
//
// Reads the raw $MFT from disk sectors and extracts names, parent FRNs,
// file sizes, hardlinks, and reparse flags in a single sequential pass.
// This replaces the combination of FSCTL_ENUM_USN_DATA + per-file
// FSCTL_GET_NTFS_FILE_RECORD, matching the approach used by "Everything".

/// Parse NTFS data-run encoding from a non-resident $DATA attribute.
/// Returns a list of (absolute_LCN, cluster_count) pairs.
fn parse_data_runs_bytes(run_data: &[u8]) -> Vec<(i64, u64)> {
    let mut runs = Vec::new();
    let mut pos = 0;
    let mut prev_lcn: i64 = 0;

    while pos < run_data.len() {
        let header = run_data[pos];
        if header == 0 {
            break;
        }
        pos += 1;

        let count_bytes = (header & 0x0F) as usize;
        let offset_bytes = ((header >> 4) & 0x0F) as usize;

        if count_bytes == 0 || pos + count_bytes + offset_bytes > run_data.len() {
            break;
        }

        // Cluster count (unsigned, little-endian).
        let mut cluster_count: u64 = 0;
        for i in 0..count_bytes {
            cluster_count |= (run_data[pos + i] as u64) << (i * 8);
        }
        pos += count_bytes;

        if offset_bytes == 0 {
            // Sparse run — no physical clusters.
            continue;
        }

        // LCN offset (signed, relative to previous LCN).
        let mut lcn_offset: i64 = 0;
        for i in 0..offset_bytes {
            lcn_offset |= (run_data[pos + i] as i64) << (i * 8);
        }
        // Sign-extend.
        if run_data[pos + offset_bytes - 1] & 0x80 != 0 {
            lcn_offset |= !0i64 << (offset_bytes * 8);
        }
        pos += offset_bytes;

        prev_lcn += lcn_offset;
        runs.push((prev_lcn, cluster_count));
    }

    runs
}

/// Read MFT record 0 and extract the $DATA attribute's data runs.
fn get_mft_data_runs(volume: HANDLE, geo: &MftGeometry) -> Result<Vec<(i64, u64)>, String> {
    let record_size = geo.bytes_per_file_record as usize;
    let mut output_buffer = vec![0u8; OUTPUT_HEADER + record_size];

    let parse_len = fetch_mft_record(volume, 0, record_size, &mut output_buffer)
        .ok_or("Failed to read MFT record 0")?;
    let record = &output_buffer[OUTPUT_HEADER..OUTPUT_HEADER + parse_len];

    if record.len() < 4 || record[0..4] != FILE_SIGNATURE {
        return Err("MFT record 0 has invalid FILE signature".to_string());
    }
    if parse_len < 0x16 {
        return Err("MFT record 0 too small".to_string());
    }

    let first_attr = u16::from_le_bytes(record[0x14..0x16].try_into().unwrap()) as usize;
    if first_attr >= parse_len || first_attr < 0x30 {
        return Err("MFT record 0 has invalid first attribute offset".to_string());
    }

    let mut offset = first_attr;
    while offset + 16 <= parse_len {
        let attr_type = u32::from_le_bytes(record[offset..offset + 4].try_into().unwrap());
        if attr_type == ATTR_TYPE_END {
            break;
        }
        let attr_len =
            u32::from_le_bytes(record[offset + 4..offset + 8].try_into().unwrap()) as usize;
        if attr_len == 0 || offset + attr_len > parse_len {
            break;
        }

        if attr_type == ATTR_TYPE_DATA {
            let non_resident = record[offset + 8];
            if non_resident != 0 && offset + 0x22 <= parse_len {
                // Non-resident $DATA — data-runs offset at +0x20.
                let run_offset_rel = u16::from_le_bytes(
                    record[offset + 0x20..offset + 0x22].try_into().unwrap(),
                ) as usize;
                let run_start = offset + run_offset_rel;
                let run_end = offset + attr_len;
                if run_start < run_end && run_start < parse_len {
                    let run_data = &record[run_start..run_end.min(parse_len)];
                    let runs = parse_data_runs_bytes(run_data);
                    if !runs.is_empty() {
                        return Ok(runs);
                    }
                }
            }
        }

        offset += attr_len;
    }

    Err("No $DATA data runs found in MFT record 0".to_string())
}

/// Apply the NTFS update-sequence (fixup) array to a raw MFT record.
/// Returns `true` if fixups were applied successfully.
fn apply_fixup(record: &mut [u8]) -> bool {
    if record.len() < 0x08 {
        return false;
    }

    let usa_offset = u16::from_le_bytes(record[0x04..0x06].try_into().unwrap()) as usize;
    let usa_count = u16::from_le_bytes(record[0x06..0x08].try_into().unwrap()) as usize;

    if usa_count < 2 || usa_offset + usa_count * 2 > record.len() {
        return false;
    }

    // NTFS always uses a 512-byte stride for the update sequence.
    const STRIDE: usize = 512;

    let check_lo = record[usa_offset];
    let check_hi = record[usa_offset + 1];

    for i in 1..usa_count {
        let pos = i * STRIDE - 2;
        if pos + 2 > record.len() {
            break;
        }

        // Verify the check value was written at the sector boundary.
        if record[pos] != check_lo || record[pos + 1] != check_hi {
            return false;
        }

        // Restore original values from the update sequence.
        let orig_offset = usa_offset + i * 2;
        if orig_offset + 2 > record.len() {
            break;
        }
        record[pos] = record[orig_offset];
        record[pos + 1] = record[orig_offset + 1];
    }

    true
}

/// Parse a single raw MFT record and populate the index.
///
/// For base records (base_record_ref == 0): extracts name, parent FRN, size,
/// is_dir, is_reparse, and inserts into the index.
///
/// For extension records (base_record_ref != 0): only looks for $DATA
/// attributes and stores the size in `extension_sizes` for later application.
fn parse_mft_record_bulk(
    record: &[u8],
    record_size: usize,
    frn: u64,
    index: &mut VolumeIndex,
    extension_sizes: &mut HashMap<u64, u64>,
    size_recheck_candidates: &mut Vec<u64>,
) {
    if record.len() < record_size || record_size < 0x30 {
        return;
    }
    if record[0..4] != FILE_SIGNATURE {
        return;
    }

    let flags = u16::from_le_bytes(record[0x16..0x18].try_into().unwrap());
    if flags & MFT_RECORD_IN_USE == 0 {
        return;
    }

    let base_ref =
        u64::from_le_bytes(record[0x20..0x28].try_into().unwrap()) & 0x0000_FFFF_FFFF_FFFF;
    let is_dir = flags & MFT_RECORD_IS_DIRECTORY != 0;

    let first_attr = u16::from_le_bytes(record[0x14..0x16].try_into().unwrap()) as usize;
    if first_attr >= record_size || first_attr < 0x30 {
        return;
    }

    // ── Extension record: only look for $DATA ──
    if base_ref != 0 {
        let mut offset = first_attr;
        while offset + 16 <= record_size {
            let attr_type = u32::from_le_bytes(record[offset..offset + 4].try_into().unwrap());
            if attr_type == ATTR_TYPE_END {
                break;
            }
            let attr_len = u32::from_le_bytes(record[offset + 4..offset + 8].try_into().unwrap())
                as usize;
            if attr_len == 0 || offset + attr_len > record_size {
                break;
            }
            if attr_type == ATTR_TYPE_DATA {
                if let Some(size) = extract_data_size_at(record, offset, record_size) {
                    extension_sizes
                        .entry(base_ref)
                        .and_modify(|existing| *existing = (*existing).max(size))
                        .or_insert(size);
                }
            }
            offset += attr_len;
        }
        return;
    }

    // ── Base record: extract name, parent, size, flags ──
    let mut best_name: Option<(String, u64, u8)> = None; // (name, parent_frn, namespace)
    let mut file_size: u64 = 0;
    let mut saw_default_data_attr = false;
    let mut has_attr_list = false;
    let mut has_reparse_attr = false;
    let mut reparse_tag: u32 = 0;
    let mut all_parents: Vec<u64> = Vec::new();

    let mut offset = first_attr;
    while offset + 16 <= record_size {
        let attr_type = u32::from_le_bytes(record[offset..offset + 4].try_into().unwrap());
        if attr_type == ATTR_TYPE_END {
            break;
        }
        let attr_len =
            u32::from_le_bytes(record[offset + 4..offset + 8].try_into().unwrap()) as usize;
        if attr_len == 0 || offset + attr_len > record_size {
            break;
        }

        match attr_type {
            ATTR_TYPE_STANDARD_INFORMATION => {
                // Resident — file attributes at content_start + 0x20.
                let non_resident = record[offset + 8];
                if non_resident == 0 && offset + 0x16 <= record_size {
                    let value_offset = u16::from_le_bytes(
                        record[offset + 0x14..offset + 0x16].try_into().unwrap(),
                    ) as usize;
                    let cs = offset + value_offset;
                    if cs + 0x24 <= record_size {
                        let file_attrs = u32::from_le_bytes(
                            record[cs + 0x20..cs + 0x24].try_into().unwrap(),
                        );
                        if file_attrs & 0x400 != 0 {
                            // FILE_ATTRIBUTE_REPARSE_POINT
                            has_reparse_attr = true;
                        }
                    }
                }
            }
            ATTR_TYPE_REPARSE_POINT => {
                // $REPARSE_POINT is always resident. First 4 bytes of the
                // value are the reparse tag (IO_REPARSE_TAG_*).
                let non_resident = record[offset + 8];
                if non_resident == 0 && offset + 0x16 <= record_size {
                    let value_offset = u16::from_le_bytes(
                        record[offset + 0x14..offset + 0x16].try_into().unwrap(),
                    ) as usize;
                    let cs = offset + value_offset;
                    if cs + 4 <= record_size {
                        reparse_tag = u32::from_le_bytes(
                            record[cs..cs + 4].try_into().unwrap(),
                        );
                    }
                }
            }
            ATTR_TYPE_FILE_NAME => {
                let non_resident = record[offset + 8];
                if non_resident == 0 && offset + 0x16 <= record_size {
                    let value_offset = u16::from_le_bytes(
                        record[offset + 0x14..offset + 0x16].try_into().unwrap(),
                    ) as usize;
                    let value_length = u32::from_le_bytes(
                        record[offset + 0x10..offset + 0x14].try_into().unwrap(),
                    ) as usize;
                    let cs = offset + value_offset;

                    if cs + 66 <= record_size && value_length >= 66 {
                        let parent_frn = u64::from_le_bytes(
                            record[cs..cs + 8].try_into().unwrap(),
                        ) & 0x0000_FFFF_FFFF_FFFF;

                        let name_chars = record[cs + 64] as usize;
                        let namespace = record[cs + 65];
                        let ns = cs + 66;
                        let nb = name_chars * 2;

                        if ns + nb <= record_size && name_chars > 0 {
                            let name_u16: Vec<u16> = record[ns..ns + nb]
                                .chunks_exact(2)
                                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                                .collect();
                            let name = String::from_utf16_lossy(&name_u16);

                            if !all_parents.contains(&parent_frn) {
                                all_parents.push(parent_frn);
                            }

                            // Namespace priority: Win32+DOS (3) > Win32 (1) > POSIX (0) > DOS (2).
                            let priority = |ns: u8| -> u8 {
                                match ns {
                                    3 => 4,
                                    1 => 3,
                                    0 => 2,
                                    2 => 1,
                                    _ => 0,
                                }
                            };
                            let should_replace = match &best_name {
                                None => true,
                                Some((_, _, prev_ns)) => priority(namespace) > priority(*prev_ns),
                            };
                            if should_replace {
                                best_name = Some((name, parent_frn, namespace));
                            }
                        }
                    }
                }
            }
            ATTR_TYPE_ATTRIBUTE_LIST => {
                has_attr_list = true;
            }
            ATTR_TYPE_DATA => {
                if let Some(size) = extract_data_size_at(record, offset, record_size) {
                    saw_default_data_attr = true;
                    file_size = size;
                }
            }
            _ => {}
        }

        offset += attr_len;
    }

    // Track reparse-point directories for DB serialization and metadata.
    // folder_size_sum no longer skips these (junctions have zero MFT children
    // anyway, so traversing them is harmless), but we still record them.
    let is_skip_reparse = has_reparse_attr && !is_cloud_reparse_tag(reparse_tag);

    // Insert if we found a valid name.
    if let Some((name, parent_frn, _)) = best_name {
        if !index.insert_record(frn, &name, parent_frn, is_dir, is_skip_reparse) {
            return; // Arena full.
        }
        if file_size > 0 {
            if let Some(rec) = index.records.get_mut(&frn) {
                rec.size = file_size;
            }
        }
        // Register extra hardlink parents.
        if all_parents.len() > 1 {
            for &parent in &all_parents {
                if parent != parent_frn {
                    let extras = index.hardlink_parents.entry(frn).or_default();
                    if !extras.contains(&parent) {
                        extras.push(parent);
                    }
                }
            }
        }

        // Recheck only files whose size could not be resolved confidently
        // during the bulk pass. Legitimate zero-byte files still have a
        // default $DATA attribute, so they are not queued.
        if !is_dir && file_size == 0 && (has_attr_list || !saw_default_data_attr) {
            size_recheck_candidates.push(frn);
        }
    }
}

/// Read the entire MFT from raw volume sectors and build a complete
/// [`VolumeIndex`] with names, parent relationships, file sizes, hardlinks,
/// and reparse-point flags — all in a single sequential I/O pass.
///
/// This replaces both `FSCTL_ENUM_USN_DATA` enumeration and per-file
/// `FSCTL_GET_NTFS_FILE_RECORD` size extraction, matching the approach
/// used by tools like *Everything*.
pub fn read_mft_bulk<F>(
    volume: HANDLE,
    drive_letter: char,
    mut on_progress: F,
) -> Result<VolumeIndex, String>
where
    F: FnMut(u64, u64),
{
    let geo = query_mft_geometry(volume)?;
    let record_size = geo.bytes_per_file_record as usize;
    let cluster_size = geo.bytes_per_cluster as usize;
    let total_mft_bytes = geo.mft_valid_data_length as u64;
    let total_records = total_mft_bytes / record_size as u64;

    eprintln!(
        "[MFT-BULK] {}:\\ MFT geometry: record={}B, sector={}B, cluster={}B, \
         mft_start_lcn={}, valid_data={:.1}MB, ~{} records",
        drive_letter,
        record_size,
        geo.bytes_per_sector,
        cluster_size,
        geo.mft_start_lcn,
        total_mft_bytes as f64 / 1_048_576.0,
        total_records
    );

    // Get the MFT's own data runs from record 0.
    let data_runs = get_mft_data_runs(volume, &geo)?;

    let total_clusters: u64 = data_runs.iter().map(|(_, c)| *c).sum();
    eprintln!(
        "[MFT-BULK] {}:\\ MFT has {} data runs, {} total clusters ({:.1} MB)",
        drive_letter,
        data_runs.len(),
        total_clusters,
        total_clusters as f64 * cluster_size as f64 / 1_048_576.0
    );

    // Open a dedicated handle for raw sequential reads (avoids moving the
    // file pointer on the handle used for IOCTL-based incremental updates).
    let read_handle = crate::usn_journal::open_volume(drive_letter)?;

    let result =
        read_mft_data(read_handle, drive_letter, &geo, &data_runs, total_records, &mut on_progress);

    crate::usn_journal::close_volume(read_handle);

    result
}

/// Inner function that performs the sequential MFT read and record parsing.
fn read_mft_data<F>(
    handle: HANDLE,
    drive_letter: char,
    geo: &MftGeometry,
    data_runs: &[(i64, u64)],
    total_records: u64,
    on_progress: &mut F,
) -> Result<VolumeIndex, String>
where
    F: FnMut(u64, u64),
{
    let record_size = geo.bytes_per_file_record as usize;
    let cluster_size = geo.bytes_per_cluster as usize;

    let mut index = VolumeIndex::new(drive_letter);
    let mut extension_sizes: HashMap<u64, u64> = HashMap::new();
    let mut size_recheck_candidates: Vec<u64> = Vec::new();

    // Read buffer: ~16 MB, aligned to both record and cluster boundaries.
    let chunk_records = (16 * 1024 * 1024) / record_size;
    let chunk_bytes = chunk_records * record_size;
    // Round down to cluster boundary for sector-aligned reads.
    let chunk_bytes = (chunk_bytes / cluster_size) * cluster_size;
    let mut read_buf = vec![0u8; chunk_bytes];

    let mut frn: u64 = 0;
    let start = std::time::Instant::now();
    let mut last_progress = std::time::Instant::now();

    on_progress(0, total_records);

    for &(lcn, cluster_count) in data_runs {
        let run_byte_offset = lcn as u64 * cluster_size as u64;
        let run_bytes = cluster_count * cluster_size as u64;

        // Seek to the start of this data run.
        let seek_result = unsafe {
            SetFilePointerEx(handle, run_byte_offset as i64, None, FILE_BEGIN)
        };
        if seek_result.is_err() {
            eprintln!(
                "[MFT-BULK] {}:\\ Seek to LCN {} (offset {}) failed, skipping run",
                drive_letter, lcn, run_byte_offset
            );
            // We still need to advance frn by the number of records in this run.
            let run_records = run_bytes / record_size as u64;
            frn += run_records;
            continue;
        }

        let mut bytes_remaining = run_bytes;
        while bytes_remaining > 0 && frn < total_records {
            let to_read = (bytes_remaining as usize).min(chunk_bytes);
            // Align down to cluster boundary.
            let to_read = (to_read / cluster_size) * cluster_size;
            if to_read == 0 {
                break;
            }

            let mut bytes_read_out: u32 = 0;
            let read_result = unsafe {
                ReadFile(
                    handle,
                    Some(&mut read_buf[..to_read]),
                    Some(&mut bytes_read_out),
                    None,
                )
            };

            if read_result.is_err() || bytes_read_out == 0 {
                let skip_records = bytes_remaining / record_size as u64;
                frn += skip_records;
                eprintln!(
                    "[MFT-BULK] {}:\\ ReadFile failed at FRN {}, skipping {} records",
                    drive_letter, frn, skip_records
                );
                break;
            }

            let bytes_read = bytes_read_out as usize;
            let usable_bytes = (bytes_read / record_size) * record_size;

            let mut buf_offset = 0;
            while buf_offset + record_size <= usable_bytes && frn < total_records {
                let record_data = &mut read_buf[buf_offset..buf_offset + record_size];

                // Apply NTFS fixup array before parsing.
                if apply_fixup(record_data) {
                    parse_mft_record_bulk(
                        record_data,
                        record_size,
                        frn,
                        &mut index,
                        &mut extension_sizes,
                        &mut size_recheck_candidates,
                    );
                }

                frn += 1;
                buf_offset += record_size;
            }

            bytes_remaining -= bytes_read as u64;

            if last_progress.elapsed() >= std::time::Duration::from_millis(200) {
                on_progress(frn, total_records);
                last_progress = std::time::Instant::now();
            }
        }
    }

    // Apply sizes from extension records to their base records.
    let mut ext_applied = 0u64;
    for (base_frn, size) in &extension_sizes {
        if let Some(rec) = index.records.get_mut(base_frn) {
            if rec.size < *size {
                rec.size = *size;
                ext_applied += 1;
            }
        }
    }

    // Recover exact sizes only for suspicious records the bulk pass could not
    // resolve. This keeps startup fast while restoring correctness for
    // $ATTRIBUTE_LIST and similar edge cases.
    let mut precise_fixed = 0u64;
    let mut precise_direct = 0u64;
    let mut precise_attr_list = 0u64;
    let mut precise_metadata = 0u64;
    let mut precise_file_id = 0u64;
    let mut precise_unresolved = 0u64;
    if !size_recheck_candidates.is_empty() {
        let mut dir_cache: HashMap<u64, String> = HashMap::with_capacity(1024);
        let mut output_buffer = vec![0u8; OUTPUT_HEADER + record_size];

        for frn in size_recheck_candidates {
            let needs_recheck = index
                .records
                .get(&frn)
                .map(|rec| !rec.is_dir && rec.size == 0)
                .unwrap_or(false);
            if !needs_recheck {
                continue;
            }

            let resolved = match resolve_file_size(handle, frn, record_size, &mut output_buffer) {
                SizeResolution::Direct(size) => {
                    precise_direct += 1;
                    Some(size)
                }
                SizeResolution::ViaAttrList(size) => {
                    precise_attr_list += 1;
                    Some(size)
                }
                SizeResolution::None => {
                    if let Some(size) = stat_file_size_fallback(&index, frn, &mut dir_cache) {
                        precise_metadata += 1;
                        Some(size)
                    } else if let Some(size) = size_by_file_id(handle, frn) {
                        precise_file_id += 1;
                        Some(size)
                    } else {
                        precise_unresolved += 1;
                        None
                    }
                }
            };

            if let Some(size) = resolved {
                if let Some(rec) = index.records.get_mut(&frn) {
                    rec.size = size;
                    precise_fixed += 1;
                }
            }
        }
    }

    on_progress(frn, total_records);

    let elapsed = start.elapsed();
    let (arena_used, _arena_cap, map_est) = index.memory_usage();
    eprintln!(
        "[MFT-BULK] {}:\\ Read {} MFT records in {:.2}s: {} files/dirs indexed, {} ext sizes applied, {} precise rechecks (direct={}, attr_list={}, metadata={}, file_id={}, unresolved={})",
        drive_letter,
        frn,
        elapsed.as_secs_f64(),
        index.records.len(),
        ext_applied,
        precise_fixed,
        precise_direct,
        precise_attr_list,
        precise_metadata,
        precise_file_id,
        precise_unresolved,
    );
    eprintln!(
        "[MFT-BULK] {}:\\ Memory: arena {:.1} MB, map ~{:.1} MB",
        drive_letter,
        arena_used as f64 / 1_048_576.0,
        map_est as f64 / 1_048_576.0,
    );

    Ok(index)
}
