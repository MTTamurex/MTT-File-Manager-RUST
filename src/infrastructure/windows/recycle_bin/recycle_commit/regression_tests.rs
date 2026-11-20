use super::{commit_item_with_cancel, metadata_record::version_two_index};
use std::cell::Cell;
use std::path::Path;

#[test]
fn version_two_index_encodes_size_time_and_terminated_utf16_path() {
    let original = Path::new(r"C:\sample\資料.txt");
    let index = version_two_index(original, 1234).unwrap();

    assert_eq!(u64::from_le_bytes(index[0..8].try_into().unwrap()), 2);
    assert_eq!(u64::from_le_bytes(index[8..16].try_into().unwrap()), 1234);
    assert_ne!(u64::from_le_bytes(index[16..24].try_into().unwrap()), 0);

    let path_length = u32::from_le_bytes(index[24..28].try_into().unwrap()) as usize;
    let path: Vec<u16> = index[28..]
        .chunks_exact(2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .collect();
    assert_eq!(path_length, path.len());
    assert_eq!(path.last(), Some(&0));
    assert_eq!(
        String::from_utf16(&path[..path.len() - 1]).unwrap(),
        original.to_string_lossy()
    );
}

#[test]
fn physical_payload_can_be_moved_back_through_the_existing_restore_api() {
    // An isolated fixture tests MTT's MoveItem path, not real bin enumeration
    // or Windows' Restore command, which require a separate integration test.
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.txt");
    let recycle_directory = temp.path().join("$Recycle.Bin").join("test-sid");
    std::fs::create_dir_all(&recycle_directory).unwrap();
    std::fs::write(&source, b"restore payload").unwrap();

    let index_data = version_two_index(&source, 15).unwrap();
    let physical =
        commit_item_with_cancel(&source, &recycle_directory, &index_data, &|| false).unwrap();
    assert!(!source.exists());

    let index_path = std::fs::read_dir(&recycle_directory)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("$I")
        })
        .unwrap();
    let content_name = physical.file_name().unwrap().to_string_lossy();
    let index_name = index_path.file_name().unwrap().to_string_lossy();
    assert_eq!(content_name.replace("$R", "$I"), index_name);
    assert_eq!(physical.extension().unwrap(), "txt");

    crate::infrastructure::windows::recycle_bin::restore_from_recycle_bin(&physical, &source)
        .unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), b"restore payload");
    assert!(!physical.exists());

    temp.close().unwrap();
}

#[test]
fn directory_move_preserves_the_subtree() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let recycle_directory = temp.path().join("bin");
    std::fs::create_dir_all(source.join("nested")).unwrap();
    std::fs::create_dir(&recycle_directory).unwrap();
    std::fs::write(source.join("nested").join("file.txt"), b"payload").unwrap();

    let index_data = version_two_index(&source, 7).unwrap();
    let physical =
        commit_item_with_cancel(&source, &recycle_directory, &index_data, &|| false).unwrap();
    assert!(!source.exists());
    assert_eq!(
        std::fs::read(physical.join("nested").join("file.txt")).unwrap(),
        b"payload"
    );
}

#[test]
fn failed_move_removes_the_index_without_touching_other_items() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("missing.txt");
    let recycle_directory = temp.path().join("bin");
    std::fs::create_dir(&recycle_directory).unwrap();
    let other = recycle_directory.join("existing.txt");
    std::fs::write(&other, b"preserve").unwrap();
    let index_data = version_two_index(&source, 0).unwrap();

    assert!(commit_item_with_cancel(&source, &recycle_directory, &index_data, &|| false).is_none());
    assert_eq!(std::fs::read(&other).unwrap(), b"preserve");
    assert_eq!(std::fs::read_dir(&recycle_directory).unwrap().count(), 1);
}

#[test]
fn index_rejects_ambiguous_restore_destinations() {
    for path in [
        "",
        "file.txt",
        r"C:relative.txt",
        r"\\server\share\file.txt",
        r"\\?\UNC\server\share\file.txt",
        r"C:\sample\..\file.txt",
        r"C:\sample\.\file.txt",
        r"C:\sample\file.txt:stream",
        "C:\\sample\\bad\0name.txt",
    ] {
        assert!(version_two_index(Path::new(path), 0).is_none());
    }
}

#[test]
fn cancellation_after_index_creation_removes_the_partial_pair() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.txt");
    let recycle_directory = temp.path().join("$Recycle.Bin").join("test-sid");
    std::fs::create_dir_all(&recycle_directory).unwrap();
    std::fs::write(&source, b"keep source").unwrap();

    let index_data = version_two_index(&source, 11).unwrap();
    let checks = Cell::new(0);
    let is_cancelled = || {
        checks.set(checks.get() + 1);
        checks.get() >= 2
    };
    assert!(
        commit_item_with_cancel(&source, &recycle_directory, &index_data, &is_cancelled,).is_none()
    );

    assert!(source.exists());
    assert_eq!(std::fs::read_dir(&recycle_directory).unwrap().count(), 0);
}

#[test]
fn version_two_index_removes_the_verbatim_drive_prefix() {
    let index = version_two_index(Path::new(r"\\?\C:\sample\file.txt"), 1).unwrap();
    let path: Vec<u16> = index[28..]
        .chunks_exact(2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .collect();
    assert_eq!(
        String::from_utf16(&path[..path.len() - 1]).unwrap(),
        r"C:\sample\file.txt"
    );
}
