//! Regression coverage for ZIP local/central header inconsistencies.

use std::io::{Cursor, Write};

use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

fn single_entry_zip() -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file("payload.exe", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"inert test content").unwrap();
    writer.finish().unwrap().into_inner()
}

#[test]
fn rejects_mismatched_local_and_central_filenames() {
    let mut bytes = single_entry_zip();
    let central = bytes
        .windows(4)
        .position(|signature| signature == b"PK\x01\x02")
        .unwrap();
    let name_start = central + 46;
    bytes[name_start..name_start + b"payload.exe".len()].copy_from_slice(b"report_.pdf");

    // Plain by_index does not enforce consistency by default. Consumers must
    // validate before trusting names or extracting entries.
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    assert!(matches!(
        archive.validate_header_consistency(),
        Err(zip::result::ZipError::InvalidArchive(_))
    ));
}

#[test]
fn reads_consistent_headers_normally() {
    let mut archive = ZipArchive::new(Cursor::new(single_entry_zip())).unwrap();
    archive.validate_header_consistency().unwrap();
    let entry = archive.by_index(0).unwrap();
    assert_eq!(entry.name().unwrap(), "payload.exe");
}

#[test]
fn oversized_symlink_target_returns_error_without_aborting() {
    use base64::{engine::general_purpose::STANDARD, Engine};

    // ZIP64 symlink with an attacker-declared uncompressed size of 2^62.
    let bytes = STANDARD
        .decode(concat!(
            "UEsDBC0AAAAAAAAAAAAAAAAAAAAAAP////8BAAAAYVBLAQItAy0AAAAAAAAAAAAAAAAAAAAAAP//",
            "//8BAAwAAAAAAAAAAAD/oQAAAABhAQAIAAAAAAAAAABAUEsFBgAAAAABAAEAOwAAAB8AAAAAAA=="
        ))
        .unwrap();
    assert_eq!(bytes.len(), 112);
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    assert!(archive.by_index(0).unwrap().is_symlink());
    let destination = tempfile::tempdir().unwrap();

    assert!(archive.extract(destination.path()).is_err());
    assert_eq!(std::fs::read_dir(destination.path()).unwrap().count(), 0);
}
