//! Local regression tests for the ZIP header consistency patch.

use crate::{rezip_parts, rezip_parts_preserving, unzip_parts};

fn mismatched_headers() -> Vec<u8> {
    let mut bytes = rezip_parts(&[("local.txt".into(), b"test content".to_vec())]).unwrap();
    let central = bytes
        .windows(4)
        .position(|signature| signature == b"PK\x01\x02")
        .unwrap();
    bytes[central + 46..central + 55].copy_from_slice(b"alias.pdf");
    bytes
}

#[test]
fn rejects_inconsistent_package_headers() {
    let error = unzip_parts(&mismatched_headers()).unwrap_err();
    assert!(error.contains("inconsistent zip headers"));
}

#[test]
fn preserving_rebuilds_inconsistent_source_instead_of_copying_it() {
    let entries = vec![("alias.pdf".into(), b"test content".to_vec())];
    let rebuilt = rezip_parts_preserving(&entries, &mismatched_headers()).unwrap();
    assert_eq!(unzip_parts(&rebuilt).unwrap(), entries);
}
