use super::*;
use std::path::PathBuf;

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("file.txt");
    let target = root.path().join("target");
    std::fs::write(&file, b"contents").unwrap();
    std::fs::create_dir(&target).unwrap();
    (root, file, target)
}

#[test]
fn disabled_engine_preserves_windows_move_dispatch() {
    let (_root, file, target) = fixture();
    let runtime = Arc::new(CopyRuntime::default());
    let (sender, receiver) = std::sync::mpsc::channel();
    let request = FileOperationRequest::move_batch(
        vec![file.clone()],
        target,
        windows::Win32::Foundation::HWND::default(),
    );
    assert!(!try_handle(&request, &sender, &runtime));
    assert!(receiver.try_recv().is_err());
    assert!(file.exists());
    assert!(runtime.jobs().is_empty());
}

#[test]
fn manual_move_dispatch_and_clipboard_move_report_actual_pairs_and_success_token() {
    for single in [false, true] {
        let (root, file, target) = fixture();
        let runtime = Arc::new(CopyRuntime::default());
        runtime.enabled.store(true, Ordering::Relaxed);
        let (sender, receiver) = std::sync::mpsc::channel();
        let request = if single {
            FileOperationRequest::Move {
                path: file.clone(),
                dest_folder: target.clone(),
                hwnd: super::super::SendHwnd(windows::Win32::Foundation::HWND::default()),
            }
        } else {
            FileOperationRequest::MoveBatch {
                paths: vec![file.clone()],
                dest_folder: target.clone(),
                hwnd: super::super::SendHwnd(windows::Win32::Foundation::HWND::default()),
                clipboard_paste_token: Some(7),
            }
        };
        assert!(try_handle(&request, &sender, &runtime));
        match receiver.try_recv().unwrap() {
            FileOperationResult::MoveBatchCompleted {
                moved_files,
                known_moved_pairs,
                clipboard_paste_token,
                ..
            } => {
                assert_eq!(moved_files.len(), 1);
                assert_eq!(known_moved_pairs.len(), 1);
                assert_eq!(known_moved_pairs[0].0, file);
                assert_eq!(known_moved_pairs[0].1, target.join("file.txt"));
                assert_eq!(clipboard_paste_token, if single { None } else { Some(7) });
            }
            _ => panic!("Expected move completion"),
        }
        assert!(matches!(
            receiver.try_recv().unwrap(),
            FileOperationResult::CopyCompleted { .. }
        ));
        assert!(receiver.try_recv().is_err());
        assert!(!file.exists());
        assert_eq!(std::fs::read(target.join("file.txt")).unwrap(), b"contents");
        let operation = runtime.jobs()[0].1.clone();
        assert!(operation.moving);
        assert_eq!(operation.outcome().unwrap().kind, OutcomeKind::Completed);
        assert_eq!(operation.progress.files_moved.load(Ordering::Relaxed), 1);
        root.close().unwrap();
    }
}

#[test]
fn skipped_or_cancelled_clipboard_move_keeps_source_and_releases_token_without_moved_pairs() {
    use mtt_copy_engine::conflicts::ConflictAction;
    for action in [ConflictAction::Skip, ConflictAction::Cancel] {
        let (root, file, target) = fixture();
        std::fs::write(target.join("file.txt"), b"old").unwrap();
        let runtime = Arc::new(CopyRuntime::default());
        runtime.enabled.store(true, Ordering::Relaxed);
        let request = FileOperationRequest::MoveBatch {
            paths: vec![file.clone()],
            dest_folder: target.clone(),
            hwnd: super::super::SendHwnd(windows::Win32::Foundation::HWND::default()),
            clipboard_paste_token: Some(9),
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        let control = runtime.clone();
        let worker = std::thread::spawn(move || try_handle(&request, &sender, &control));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some((operation, conflict)) =
                runtime.jobs().into_iter().find_map(|(_, operation)| {
                    operation
                        .progress
                        .conflicts
                        .pending()
                        .map(|conflict| (operation, conflict))
                })
            {
                assert!(operation
                    .progress
                    .conflicts
                    .respond(conflict.id, action, false));
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(worker.join().unwrap());
        match receiver.try_recv().unwrap() {
            FileOperationResult::MoveBatchCompleted {
                moved_files,
                known_moved_pairs,
                clipboard_paste_token,
                ..
            } => {
                assert!(moved_files.is_empty());
                assert!(known_moved_pairs.is_empty());
                assert!(clipboard_paste_token.is_none());
            }
            _ => panic!("Expected move completion"),
        }
        assert!(matches!(
            receiver.try_recv().unwrap(),
            FileOperationResult::CopyCompleted { .. }
        ));
        assert!(matches!(
            receiver.try_recv().unwrap(),
            FileOperationResult::ClipboardMoveFailed { token: 9, .. }
        ));
        assert!(receiver.try_recv().is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"contents");
        assert_eq!(std::fs::read(target.join("file.txt")).unwrap(), b"old");
        assert_eq!(
            runtime.jobs()[0].1.outcome().unwrap().kind,
            if action == ConflictAction::Cancel {
                OutcomeKind::Cancelled
            } else {
                OutcomeKind::Completed
            }
        );
        root.close().unwrap();
    }
}
