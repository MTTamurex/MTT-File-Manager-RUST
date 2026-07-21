use super::*;
use std::os::windows::fs::OpenOptionsExt;

fn protected_source(path: &Path) -> std::fs::File {
    std::fs::OpenOptions::new()
        .access_mode(FILE_GENERIC_READ.0 | DELETE.0)
        .share_mode(FILE_SHARE_READ.0)
        .open(path)
        .expect("open protected source")
}

#[test]
fn verified_copy_exposes_only_valid_final_contents() {
    let source_parent = tempfile::tempdir().expect("create source parent");
    let destination_parent = tempfile::tempdir().expect("create destination parent");
    let source = source_parent.path().join("report.pdf");
    let destination = destination_parent.path().join("report.pdf");
    let contents = vec![0x5a; 2 * 1024 * 1024];
    std::fs::write(&source, &contents).expect("create source");
    let mut source_stream_name = source.as_os_str().to_os_string();
    source_stream_name.push(":organizer-test");
    std::fs::write(Path::new(&source_stream_name), b"alternate stream")
        .expect("create alternate stream");
    let mut guard = protected_source(&source);

    move_across_volumes_verified(&mut guard, &destination).expect("verified move");
    drop(guard);

    assert!(!source.exists());
    assert_eq!(
        std::fs::read(&destination).expect("read destination"),
        contents
    );
    let mut destination_stream_name = destination.as_os_str().to_os_string();
    destination_stream_name.push(":organizer-test");
    assert_eq!(
        std::fs::read(Path::new(&destination_stream_name)).expect("read alternate stream"),
        b"alternate stream"
    );
}

#[test]
fn verified_copy_never_replaces_an_existing_final_destination() {
    let source_parent = tempfile::tempdir().expect("create source parent");
    let destination_parent = tempfile::tempdir().expect("create destination parent");
    let source = source_parent.path().join("report.pdf");
    let destination = destination_parent.path().join("report.pdf");
    std::fs::write(&source, b"source").expect("create source");
    std::fs::write(&destination, b"destination").expect("create destination");
    let mut guard = protected_source(&source);

    let result = move_across_volumes_verified(&mut guard, &destination);
    drop(guard);

    assert!(result.is_err());
    assert_eq!(std::fs::read(source).expect("source remains"), b"source");
    assert_eq!(
        std::fs::read(destination).expect("destination remains"),
        b"destination"
    );
}

#[test]
fn protected_source_handle_prevents_path_replacement() {
    let directory = tempfile::tempdir().expect("create directory");
    let source = directory.path().join("source.txt");
    let displaced = directory.path().join("displaced.txt");
    std::fs::write(&source, b"source").expect("create source");
    let guard = protected_source(&source);

    assert!(std::fs::rename(&source, &displaced).is_err());
    assert_eq!(std::fs::read(&source).expect("source remains"), b"source");
    drop(guard);
}

#[test]
fn protected_temporary_handle_prevents_late_writes() {
    let directory = tempfile::tempdir().expect("create directory");
    let path = directory.path().join("temporary.txt");
    std::fs::write(&path, b"validated").expect("create temporary file");
    let _guard = std::fs::OpenOptions::new()
        .access_mode(FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0 | DELETE.0)
        .share_mode(FILE_SHARE_READ.0)
        .open(&path)
        .expect("protect temporary file");

    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
    assert_eq!(std::fs::read(&path).expect("contents remain"), b"validated");
}

#[test]
fn handle_rename_preserves_a_long_file_name_exactly() {
    let source_parent = tempfile::tempdir().expect("create source parent");
    let destination_parent = tempfile::tempdir().expect("create destination parent");
    let source = source_parent.path().join("source.mp4");
    let expected_name = "Miss Monique - Live @ Radio Intense 11.06.2021 [Progressive House - Melodic Techno DJ Mix] 4K.mp4";
    let destination = destination_parent.path().join(expected_name);
    std::fs::write(&source, b"video contents").expect("create source");

    move_file_without_replace(&source, &destination).expect("rename long file name");

    let names: Vec<_> = std::fs::read_dir(destination_parent.path())
        .expect("read destination directory")
        .map(|entry| entry.expect("directory entry").file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from(expected_name)]);
    assert_eq!(
        std::fs::read(destination).expect("read renamed file"),
        b"video contents"
    );
}
