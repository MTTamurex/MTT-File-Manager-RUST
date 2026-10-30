use super::*;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::windows::io::AsRawHandle,
};
use windows::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};

fn record(sequence: u16) -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes[..4].copy_from_slice(&FILE_SIGNATURE);
    bytes[0x10..0x12].copy_from_slice(&sequence.to_le_bytes());
    bytes[0x16..0x18].copy_from_slice(&MFT_RECORD_IN_USE.to_le_bytes());
    bytes
}

#[test]
fn normalized_index_key_must_restore_the_ntfs_sequence_for_open_file_by_id() {
    for sequence in [1, 12, u16::MAX] {
        assert_eq!(
            reference_from_record(42, &record(sequence)),
            Some(((sequence as u64) << 48) | 42)
        );
    }
}

#[test]
fn invalid_free_extension_and_truncated_records_cannot_resolve_a_reference() {
    assert!(reference_from_record(42, &[]).is_none());
    assert!(reference_from_record(1 << 48, &record(3)).is_none());
    let mut bytes = record(3);
    bytes[0] = 0;
    assert!(reference_from_record(42, &bytes).is_none());
    let mut bytes = record(3);
    bytes[0x16..0x18].fill(0);
    assert!(reference_from_record(42, &bytes).is_none());
    let mut bytes = record(3);
    bytes[0x20..0x28].copy_from_slice(&5u64.to_le_bytes());
    assert!(reference_from_record(42, &bytes).is_none());
}

fn identity(file: &File) -> (HANDLE, u64) {
    let handle = HANDLE(file.as_raw_handle());
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(handle, &mut info) }.unwrap();
    (
        handle,
        ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    )
}

#[test]
fn windows_queries_complete_reference_but_rejects_the_old_normalized_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.bin");
    std::fs::write(&path, vec![7; 12345]).unwrap();
    let file = File::open(&path).unwrap();
    let (hint, full) = identity(&file);
    assert_ne!(full >> 48, 0);
    assert!(read_by_reference(hint, full & 0x0000_FFFF_FFFF_FFFF).is_none());
    let restored =
        reference_from_record(full & 0x0000_FFFF_FFFF_FFFF, &record((full >> 48) as u16)).unwrap();
    let metrics = read_by_reference(hint, restored).expect("complete reference must resolve");
    assert_eq!(metrics.logical_size, 12345);
    assert!(metrics.allocated_size >= metrics.logical_size);
}

#[test]
fn live_metrics_follow_growth_shrink_empty_files_and_named_stream_allocation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.bin");
    let mut writer = File::create(&path).unwrap();
    writer.write_all(&vec![7; 16384]).unwrap();
    drop(writer);
    let file = File::open(&path).unwrap();
    let (hint, reference) = identity(&file);
    let initial = read_by_reference(hint, reference).unwrap();
    assert_eq!(initial.logical_size, 16384);
    let ads = path.with_file_name("file.bin:extra");
    std::fs::write(ads, vec![9; 8192]).unwrap();
    let with_ads = read_by_reference(hint, reference).unwrap();
    assert_eq!(with_ads.logical_size, initial.logical_size);
    assert!(with_ads.allocated_size > initial.allocated_size);
    let writer = OpenOptions::new().write(true).open(&path).unwrap();
    for size in [32768, 4096, 0] {
        writer.set_len(size).unwrap();
        assert_eq!(
            read_by_reference(hint, reference).unwrap().logical_size,
            size
        );
    }
}

#[test]
fn malformed_stream_info_is_unknown_not_zero() {
    assert!(parse_streams(&[], 17).is_none());
    let mut bytes = vec![0u8; 24];
    bytes[8..16].copy_from_slice(&(-1i64).to_le_bytes());
    assert!(parse_streams(&bytes, 17).is_none());
}

fn stream(name: &str, size: i64, allocated: i64) -> Vec<u8> {
    let name: Vec<_> = name.encode_utf16().collect();
    let mut bytes = vec![0; 24 + name.len() * 2];
    bytes[4..8].copy_from_slice(&((name.len() * 2) as u32).to_le_bytes());
    bytes[8..16].copy_from_slice(&size.to_le_bytes());
    bytes[16..24].copy_from_slice(&allocated.to_le_bytes());
    for (chunk, ch) in bytes[24..].chunks_exact_mut(2).zip(name) {
        chunk.copy_from_slice(&ch.to_le_bytes());
    }
    bytes
}

#[test]
fn incomplete_or_changing_stream_metadata_is_rejected_not_cached_as_zero() {
    assert!(parse_streams(&[0; 128], 0).is_none());
    assert!(parse_streams(&stream(":extra:$DATA", 10, 4096), 17).is_none());
    assert!(parse_streams(&stream("::$DATA", 18, 4096), 17).is_none());
    assert_eq!(
        parse_streams(&stream("::$DATA", 0, 0), 0),
        Some(FileMetrics::default())
    );
    let mut truncated = stream("::$DATA", 17, 4096);
    truncated.pop();
    assert!(parse_streams(&truncated, 17).is_none());
}

#[test]
fn stream_offsets_cannot_overlap_names_or_repeat_the_default_stream() {
    let first = stream("::$DATA", 17, 4096);
    let mut duplicate = first.clone();
    duplicate.resize(duplicate.len().next_multiple_of(8), 0);
    let next = duplicate.len() as u32;
    duplicate[..4].copy_from_slice(&next.to_le_bytes());
    duplicate.extend(first);
    assert!(parse_streams(&duplicate, 17).is_none());
    duplicate[..4].copy_from_slice(&24u32.to_le_bytes());
    assert!(parse_streams(&duplicate, 17).is_none());
}
