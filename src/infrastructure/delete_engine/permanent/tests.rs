use super::*;
use crate::infrastructure::copy_engine::CopyRuntime;
use std::sync::atomic::Ordering;

#[test]
fn removes_nested_synthetic_files_and_folders_without_an_enumeration_pass() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("nested/empty")).unwrap();
    fs::write(source.join("file.txt"), b"synthetic").unwrap();
    fs::write(source.join("nested/image.jpg"), b"synthetic").unwrap();
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete(vec![source.clone()], true);
    delete(&source, &job.operation).unwrap();
    assert!(!source.exists());
    assert_eq!(
        job.operation
            .deletion
            .as_ref()
            .unwrap()
            .removed
            .load(Ordering::Relaxed),
        5
    );
    assert_eq!(
        job.operation
            .deletion
            .as_ref()
            .unwrap()
            .failures
            .load(Ordering::Relaxed),
        0
    );
    temp.close().unwrap();
}

#[test]
fn cancellation_leaves_remaining_items_and_does_not_restore_previous_deletions() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first.txt");
    let second = temp.path().join("second.txt");
    fs::write(&first, b"synthetic").unwrap();
    fs::write(&second, b"synthetic").unwrap();
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete(vec![first.clone(), second.clone()], true);
    delete(&first, &job.operation).unwrap();
    job.operation.progress.cancel();
    assert_eq!(
        delete(&second, &job.operation).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
    assert!(!first.exists());
    assert!(second.exists());
    temp.close().unwrap();
}

#[test]
fn locked_file_reports_failure_and_preserves_its_parent() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let path = source.join("file.txt");
    fs::write(&path, b"synthetic").unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .open(&path)
        .unwrap();
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete(vec![source.clone()], true);
    assert!(delete(&source, &job.operation).is_err());
    assert!(path.exists());
    assert!(
        job.operation
            .deletion
            .as_ref()
            .unwrap()
            .failures
            .load(Ordering::Relaxed)
            > 0
    );
    drop(lock);
    temp.close().unwrap();
}

#[test]
fn nested_junction_is_removed_without_touching_its_target() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    let protected = target.join("file.txt");
    fs::write(&protected, b"synthetic protected data").unwrap();
    let junction = source.join("junction");
    let result = std::process::Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&target)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "Could not create synthetic junction"
    );
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete(vec![source.clone()], true);
    delete(&source, &job.operation).unwrap();
    assert!(!source.exists());
    assert_eq!(fs::read(&protected).unwrap(), b"synthetic protected data");
    temp.close().unwrap();
}

#[test]
fn reparse_ancestor_is_refused_before_removing_children() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target");
    fs::create_dir(&target).unwrap();
    let protected = target.join("file.txt");
    fs::write(&protected, b"synthetic protected data").unwrap();
    let junction = temp.path().join("junction");
    let result = std::process::Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&target)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "Could not create synthetic junction"
    );
    let path = junction.join("file.txt");
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete(vec![path.clone()], true);
    assert_eq!(
        delete(&path, &job.operation).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(fs::read(&protected).unwrap(), b"synthetic protected data");
    fs::remove_dir(&junction).unwrap();
    temp.close().unwrap();
}

#[test]
fn cancellation_inside_a_tree_reports_only_completed_removals() {
    use std::sync::{Arc, Mutex};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    for index in 0..3 {
        fs::write(source.join(format!("file-{index}.txt")), b"synthetic").unwrap();
    }
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete(vec![source.clone()], true);
    let reported = Arc::new(Mutex::new(Vec::new()));
    let observer: crate::infrastructure::delete_engine::RemovalObserver = {
        let operation = job.operation.clone();
        let reported = reported.clone();
        Arc::new(move |path| {
            reported.lock().unwrap().push(path.to_path_buf());
            operation.progress.cancel();
        })
    };
    assert_eq!(
        delete_with_observer(&source, &job.operation, &observer)
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
    let reported = reported.lock().unwrap();
    assert_eq!(reported.len(), 1);
    assert!(!reported[0].exists());
    assert_eq!(fs::read_dir(&source).unwrap().count(), 2);
    assert_eq!(
        job.operation
            .deletion
            .as_ref()
            .unwrap()
            .removed
            .load(Ordering::Relaxed),
        1
    );
    temp.close().unwrap();
}
