use super::*;
use std::fs;

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    (root, source, target)
}

#[test]
fn copies_tree_and_empty_directories_without_changing_source() {
    let (_root, source, target) = fixture();
    fs::create_dir(source.join("empty")).unwrap();
    fs::write(source.join("file.txt"), b"contents").unwrap();
    fs::write(source.join("empty.txt"), b"").unwrap();
    let progress = Progress::default();
    let summary = copy(std::slice::from_ref(&source), &target, &progress).unwrap();
    assert_eq!(summary.failures, 0);
    assert_eq!(summary.files_copied, 2);
    assert!(!summary.cancelled);
    assert_eq!(
        fs::read(target.join("source/file.txt")).unwrap(),
        b"contents"
    );
    assert_eq!(fs::read(source.join("file.txt")).unwrap(), b"contents");
    assert!(target.join("source/empty").is_dir());
    assert_eq!(progress.total_bytes.load(Ordering::Relaxed), 8);
    assert_eq!(progress.bytes_copied.load(Ordering::Relaxed), 8);
    assert!(progress.active_files().is_empty());
}

#[test]
fn refuses_same_sized_existing_file_without_changing_either_file() {
    let (_root, source, target) = fixture();
    let file = source.join("file.txt");
    fs::write(&file, b"source").unwrap();
    fs::write(target.join("file.txt"), b"target").unwrap();
    let error = copy(std::slice::from_ref(&file), &target, &Progress::default()).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(file).unwrap(), b"source");
    assert_eq!(fs::read(target.join("file.txt")).unwrap(), b"target");
}

#[test]
fn existing_directory_is_not_merged() {
    let (_root, source, target) = fixture();
    fs::write(source.join("file.txt"), b"source").unwrap();
    fs::create_dir(target.join("source")).unwrap();
    assert!(copy(&[source], &target, &Progress::default()).is_err());
    assert!(!target.join("source/file.txt").exists());
}

#[test]
fn refuses_recursive_and_self_copy_before_writing() {
    let (_root, source, _target) = fixture();
    let child = source.join("child");
    fs::create_dir(&child).unwrap();
    assert!(copy(std::slice::from_ref(&source), &source, &Progress::default()).is_err());
    assert!(copy(std::slice::from_ref(&source), &child, &Progress::default()).is_err());
    assert!(!child.join("source").exists());
}

#[test]
fn cancellation_during_preflight_writes_nothing() {
    let (_root, source, target) = fixture();
    let progress = Progress::default();
    progress.cancel.store(true, Ordering::Relaxed);
    assert_eq!(
        copy(&[source], &target, &progress).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(fs::read_dir(target).unwrap().count(), 0);
}

#[test]
fn duplicate_names_and_missing_sources_write_nothing() {
    let (root, source, target) = fixture();
    let other = root.path().join("other");
    fs::create_dir(&other).unwrap();
    fs::write(source.join("file.txt"), b"one").unwrap();
    fs::write(other.join("file.txt"), b"two").unwrap();
    assert!(copy(
        &[source.join("file.txt"), other.join("file.txt")],
        &target,
        &Progress::default()
    )
    .is_err());
    assert!(copy(
        &[source, other.join("missing.txt")],
        &target,
        &Progress::default()
    )
    .is_err());
    assert_eq!(fs::read_dir(target).unwrap().count(), 0);
}

#[test]
fn copies_large_and_unicode_files_and_leaves_no_staging_files() {
    let (_root, source, target) = fixture();
    let file = source.join("unicode-ç-日本.txt");
    let contents = vec![37u8; 2 * 1024 * 1024 + 13];
    fs::write(&file, &contents).unwrap();
    let summary = copy(std::slice::from_ref(&file), &target, &Progress::default()).unwrap();
    assert_eq!(summary.files_copied, 1);
    assert_eq!(
        fs::read(target.join(file.file_name().unwrap())).unwrap(),
        contents
    );
    assert_eq!(fs::read_dir(target).unwrap().count(), 1);
}

#[test]
fn target_appearing_after_preflight_is_never_replaced() {
    let (_root, source, target) = fixture();
    let file = source.join("file.txt");
    fs::write(&file, b"source").unwrap();
    let plan = plan::build(std::slice::from_ref(&file), &target, &Progress::default()).unwrap();
    fs::write(target.join("file.txt"), b"target").unwrap();
    assert!(platform::copy_file(&plan.files[0], &Progress::default()).is_err());
    assert_eq!(fs::read(target.join("file.txt")).unwrap(), b"target");
    assert_eq!(fs::read(file).unwrap(), b"source");
    assert_eq!(fs::read_dir(target).unwrap().count(), 1);
}

#[test]
fn changed_source_is_not_published() {
    let (_root, source, target) = fixture();
    let file = source.join("file.txt");
    fs::write(&file, b"source").unwrap();
    let plan = plan::build(std::slice::from_ref(&file), &target, &Progress::default()).unwrap();
    fs::write(&file, b"changed source").unwrap();
    let progress = Progress::default();
    assert!(platform::copy_file(&plan.files[0], &progress).is_err());
    assert_eq!(progress.bytes_copied.load(Ordering::Relaxed), 0);
    assert_eq!(fs::read_dir(target).unwrap().count(), 0);
}

#[cfg(windows)]
#[test]
fn locked_source_failure_removes_staging_file() {
    use std::os::windows::fs::OpenOptionsExt;
    let (_root, source, target) = fixture();
    let file = source.join("file.txt");
    fs::write(&file, b"contents").unwrap();
    let _lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&file)
        .unwrap();
    let progress = Progress::default();
    let summary = copy(&[file], &target, &progress).unwrap();
    assert_eq!(summary.failures, 1);
    assert_eq!(summary.files_copied, 0);
    assert!(progress.active_files().is_empty());
    assert!(summary.first_error.is_some());
    assert_eq!(progress.bytes_copied.load(Ordering::Relaxed), 0);
    assert_eq!(fs::read_dir(target).unwrap().count(), 0);
}

#[cfg(windows)]
#[test]
fn readonly_source_is_copied_and_failed_publication_cleans_staging() {
    let (_root, source, target) = fixture();
    let file = source.join("file.txt");
    fs::write(&file, b"contents").unwrap();
    let mut permissions = fs::metadata(&file).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&file, permissions).unwrap();
    let summary = copy(&[file], &target, &Progress::default()).unwrap();
    assert_eq!(summary.failures, 0);
    assert!(fs::metadata(target.join("file.txt"))
        .unwrap()
        .permissions()
        .readonly());

    let staged = staging::StagingFile::new(&target).unwrap();
    let mut permissions = fs::metadata(staged.path()).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(staged.path(), permissions).unwrap();
    assert!(staged.publish(&target.join("file.txt")).is_err());
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    assert_eq!(fs::read(target.join("file.txt")).unwrap(), b"contents");
}

#[cfg(windows)]
#[test]
fn junction_in_tree_is_rejected_without_writes() {
    let (root, source, target) = fixture();
    let outside = root.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let link = source.join("junction");
    // Directory junctions do not require developer mode/elevation.
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&outside)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(copy(&[source], &target, &Progress::default()).is_err());
    assert_eq!(fs::read_dir(target).unwrap().count(), 0);
    fs::remove_dir(link).unwrap();
}
