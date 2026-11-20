use super::StagingFile;
use std::fs;

#[test]
fn disarming_publication_never_removes_a_reused_temporary_name() {
    let root = tempfile::tempdir().unwrap();
    let mut staging = StagingFile::new(root.path()).unwrap();
    let old_path = staging.path().to_owned();
    let destination = root.path().join("file.txt");
    fs::write(&old_path, b"contents").unwrap();
    fs::rename(&old_path, &destination).unwrap();
    fs::write(&old_path, b"unrelated").unwrap();

    staging.disarm_cleanup();
    drop(staging);

    assert_eq!(fs::read(&old_path).unwrap(), b"unrelated");
    assert_eq!(fs::read(&destination).unwrap(), b"contents");
    root.close().unwrap();
}

#[test]
fn failed_publication_cleans_staging_without_touching_existing_destination() {
    let root = tempfile::tempdir().unwrap();
    let staging = StagingFile::new(root.path()).unwrap();
    let old_path = staging.path().to_owned();
    let destination = root.path().join("file.txt");
    fs::write(&old_path, b"contents").unwrap();
    fs::write(&destination, b"existing").unwrap();

    assert!(staging.publish(&destination).is_err());
    assert!(!old_path.exists());
    assert_eq!(fs::read(&destination).unwrap(), b"existing");
    root.close().unwrap();
}

#[test]
fn staging_guard_refuses_an_existing_writer() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let root = tempfile::tempdir().unwrap();
    let staging = StagingFile::new(root.path()).unwrap();
    let writer = fs::OpenOptions::new()
        .write(true)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
        .open(staging.path())
        .unwrap();
    let error = crate::replacement::Snapshot::protect_staging(staging.path()).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(32));
    drop(writer);
    drop(staging);
    root.close().unwrap();
}
