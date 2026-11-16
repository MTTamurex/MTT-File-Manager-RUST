use super::*;

#[test]
fn thousands_of_removals_have_bounded_progress_and_one_terminal_refresh() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    for index in 0..1024 {
        std::fs::write(source.join(format!("file-{index}.txt")), b"synthetic").unwrap();
    }
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
    let mut removed = HashSet::new();
    let mut terminal = 0;
    for event in receiver.try_iter() {
        match event {
            FileOperationResult::DeleteProgress { deleted_paths, .. } => {
                assert_eq!(terminal, 0);
                assert!(deleted_paths.len() <= 128);
                for path in deleted_paths {
                    assert!(removed.insert(path));
                }
            }
            FileOperationResult::DeleteCompleted {
                deleted_paths,
                parent_folders,
            } => {
                terminal += 1;
                assert!(deleted_paths.is_empty());
                assert_eq!(parent_folders, vec![temp.path().to_path_buf()]);
            }
            _ => panic!("Unexpected deletion event"),
        }
    }
    assert_eq!(removed.len(), 1025);
    assert_eq!(terminal, 1);
    assert!(!runtime.delete_activity.affects(&source));
    assert!(!source.exists());
}
