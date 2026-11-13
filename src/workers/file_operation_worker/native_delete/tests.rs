use super::*;
use windows::Win32::Foundation::HWND;

#[test]
fn default_engine_leaves_all_deletion_requests_unchanged() {
    let runtime = Arc::new(CopyRuntime::default());
    let (sender, receiver) = std::sync::mpsc::channel();
    let paths = vec![PathBuf::from(r"C:\source\file.txt")];
    for request in [
        FileOperationRequest::delete(paths.clone(), HWND::default()),
        FileOperationRequest::delete_permanently(paths.clone(), HWND::default()),
        FileOperationRequest::search_delete(paths, false, HWND::default()),
    ] {
        assert!(!try_handle(&request, &sender, &runtime));
    }
    assert!(runtime.jobs().is_empty());
    assert!(receiver.try_recv().is_err());
}

#[test]
fn deduplicates_parent_and_child_selections_before_execution() {
    assert_eq!(
        unique_roots(vec![
            PathBuf::from("source/folder/file.txt"),
            PathBuf::from("source/folder"),
            PathBuf::from("source/folder")
        ]),
        vec![PathBuf::from("source/folder")]
    );
}

#[test]
fn virtual_items_and_unrelated_requests_keep_existing_semantics() {
    let runtime = Arc::new(CopyRuntime::default());
    runtime.enabled.store(true, Ordering::Relaxed);
    let (sender, _) = std::sync::mpsc::channel();
    assert!(!try_handle(
        &FileOperationRequest::delete(
            vec![PathBuf::from("shell:RecycleBinFolder")],
            HWND::default()
        ),
        &sender,
        &runtime
    ));
    assert!(!try_handle(
        &FileOperationRequest::copy_batch(
            vec![PathBuf::from(r"C:\source\file.txt")],
            PathBuf::from(r"D:\target"),
            HWND::default()
        ),
        &sender,
        &runtime
    ));
    assert!(runtime.jobs().is_empty());
}

#[test]
fn permanent_delete_requires_confirmation_and_reports_actual_removed_entries() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir_all(source.join("nested")).unwrap();
    std::fs::write(source.join("nested/file.txt"), b"synthetic").unwrap();
    let request = FileOperationRequest::delete_permanently(vec![source.clone()], HWND::default());
    let runtime = Arc::new(CopyRuntime::default());
    runtime.enabled.store(true, Ordering::Relaxed);
    let (sender, receiver) = std::sync::mpsc::channel();
    assert!(handle_with_confirmation(
        &request,
        &sender,
        &runtime,
        |_, kind, paths| {
            assert_eq!(kind, confirmation::Kind::Permanent);
            assert_eq!(paths.len(), 1);
            false
        }
    ));
    assert!(source.join("nested/file.txt").exists());
    assert_eq!(
        runtime.jobs()[0].1.outcome().unwrap().kind,
        OutcomeKind::Cancelled
    );
    assert!(receiver.try_recv().is_err());
    assert!(handle_with_confirmation(
        &request,
        &sender,
        &runtime,
        |_, _, _| true
    ));
    assert!(!source.exists());
    let FileOperationResult::DeleteCompleted {
        deleted_paths,
        parent_folders,
    } = receiver.try_recv().unwrap()
    else {
        panic!("Expected deletion result");
    };
    assert_eq!(deleted_paths.len(), 3);
    assert!(deleted_paths.contains(&source));
    assert!(deleted_paths.contains(&source.join("nested/file.txt")));
    assert!(parent_folders.contains(&temp.path().to_path_buf()));
    assert_eq!(
        runtime.jobs()[1].1.outcome().unwrap().kind,
        OutcomeKind::Completed
    );
    temp.close().unwrap();
}

#[test]
fn recycle_bin_storage_keeps_shell_metadata_semantics() {
    let runtime = Arc::new(CopyRuntime::default());
    runtime.enabled.store(true, Ordering::Relaxed);
    let (sender, _) = std::sync::mpsc::channel();
    let request = FileOperationRequest::delete_permanently(
        vec![PathBuf::from(r"C:\$Recycle.Bin\entry")],
        HWND::default(),
    );
    assert!(!try_handle(&request, &sender, &runtime));
    assert!(runtime.jobs().is_empty());
}

#[test]
fn declining_confirmation_never_deletes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("file.txt");
    std::fs::write(&path, b"synthetic").unwrap();
    let runtime = Arc::new(CopyRuntime::default());
    runtime.enabled.store(true, Ordering::Relaxed);
    let (sender, receiver) = std::sync::mpsc::channel();
    let request = FileOperationRequest::delete_permanently(vec![path.clone()], HWND::default());
    assert!(handle_with_confirmation(
        &request,
        &sender,
        &runtime,
        |_, _, _| false
    ));
    assert!(path.exists());
    assert_eq!(
        runtime.jobs()[0].1.outcome().unwrap().kind,
        OutcomeKind::Cancelled
    );
    assert!(receiver.try_recv().is_err());
    temp.close().unwrap();
}

#[test]
fn partial_failure_refreshes_parent_without_reporting_remaining_root_deleted() {
    use std::os::windows::fs::OpenOptionsExt;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let locked = source.join("locked.txt");
    std::fs::write(&locked, b"synthetic").unwrap();
    let removed = source.join("other.txt");
    std::fs::write(&removed, b"synthetic").unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&locked)
        .unwrap();
    let runtime = Arc::new(CopyRuntime::default());
    runtime.enabled.store(true, Ordering::Relaxed);
    let (sender, receiver) = std::sync::mpsc::channel();
    let request = FileOperationRequest::delete_permanently(vec![source.clone()], HWND::default());
    assert!(handle_with_confirmation(
        &request,
        &sender,
        &runtime,
        |_, _, _| true
    ));
    assert!(!removed.exists());
    assert!(locked.exists());
    let FileOperationResult::DeleteCompleted {
        deleted_paths,
        parent_folders,
    } = receiver.try_recv().unwrap()
    else {
        panic!("Expected deletion result");
    };
    assert_eq!(deleted_paths, vec![removed]);
    assert!(!deleted_paths.contains(&source));
    assert_eq!(parent_folders.len(), 1);
    assert_eq!(parent_folders[0], source);
    assert_eq!(
        runtime.jobs()[0].1.outcome().unwrap().kind,
        OutcomeKind::Failed
    );
    drop(lock);
    temp.close().unwrap();
}

#[test]
fn normal_and_permanent_delete_wait_for_mtt_before_any_filesystem_change() {
    use crate::infrastructure::delete_engine::confirmation::{tests::wait_for_request, Kind};
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("file.txt");
    std::fs::write(&path, b"synthetic").unwrap();
    for permanently in [false, true] {
        let runtime = Arc::new(CopyRuntime::default());
        runtime.enabled.store(true, Ordering::Relaxed);
        let (sender, _) = std::sync::mpsc::channel();
        let request = if permanently {
            FileOperationRequest::delete_permanently(vec![path.clone()], HWND::default())
        } else {
            FileOperationRequest::delete(vec![path.clone()], HWND::default())
        };
        let worker_runtime = runtime.clone();
        let worker = std::thread::spawn(move || try_handle(&request, &sender, &worker_runtime));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let operation = loop {
            if let Some((_, operation)) = runtime.jobs().into_iter().next() {
                break operation;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        };
        let request = wait_for_request(&operation);
        assert_eq!(
            request.kind,
            if permanently {
                Kind::Permanent
            } else {
                Kind::Recycle
            }
        );
        assert!(path.exists());
        assert!(!worker.is_finished());
        operation
            .deletion
            .as_ref()
            .unwrap()
            .confirmation
            .respond(request.id, false);
        assert!(worker.join().unwrap());
        assert!(path.exists());
        assert_eq!(operation.outcome().unwrap().kind, OutcomeKind::Cancelled);
    }
    temp.close().unwrap();
}
