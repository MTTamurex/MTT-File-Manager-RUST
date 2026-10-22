use super::*;
use std::sync::Mutex;

#[test]
fn shallow_subfolders_publish_progress_before_the_final_total() {
    let root = tempfile::tempdir().unwrap();
    for name in ["folder-a", "folder-b", "folder-c"] {
        let child = root.path().join(name);
        std::fs::create_dir(&child).unwrap();
        std::fs::write(child.join("file.txt"), [0u8; 10]).unwrap();
    }
    let progress = Mutex::new(Vec::new());
    let result =
        calculate_folder_size_parallel(root.path(), &Arc::new(AtomicBool::new(false)), |size| {
            progress.lock().unwrap().push(size)
        })
        .unwrap();

    assert_eq!(result.total_size, 30);
    assert_eq!(result.file_count, 3);
    assert_eq!(result.folder_count, 3);
    let progress = progress.into_inner().unwrap();
    assert!(progress.iter().any(|&size| size > 0 && size < 30));
    assert_eq!(progress.last(), Some(&30));
}

#[test]
fn parallel_leaf_scans_publish_their_accumulated_sizes() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("folder");
    std::fs::create_dir(&parent).unwrap();
    for name in ["folder-a", "folder-b", "folder-c"] {
        let child = parent.join(name);
        std::fs::create_dir(&child).unwrap();
        std::fs::write(child.join("file.txt"), [0u8; 10]).unwrap();
    }
    let progress = Mutex::new(Vec::new());
    let result =
        calculate_folder_size_parallel(root.path(), &Arc::new(AtomicBool::new(false)), |size| {
            progress.lock().unwrap().push(size)
        })
        .unwrap();

    assert_eq!(result.total_size, 30);
    assert_eq!(result.file_count, 3);
    assert_eq!(result.folder_count, 4);
    let progress = progress.into_inner().unwrap();
    assert!(progress.iter().any(|&size| size > 0));
    assert_eq!(progress.iter().max(), Some(&result.total_size));
}

#[test]
fn large_flat_directory_reports_progress_before_enumeration_finishes() {
    let root = tempfile::tempdir().unwrap();
    for index in 0..600 {
        std::fs::write(root.path().join(format!("file-{index}.txt")), [0u8; 1]).unwrap();
    }
    let progress = Mutex::new(Vec::new());
    let result =
        calculate_folder_size_parallel(root.path(), &Arc::new(AtomicBool::new(false)), |size| {
            progress.lock().unwrap().push(size)
        })
        .unwrap();

    assert_eq!(result.total_size, 600);
    assert_eq!(result.file_count, 600);
    assert!(progress
        .into_inner()
        .unwrap()
        .iter()
        .any(|&size| size > 0 && size < 600));
}

#[test]
fn large_directory_scan_observes_cancellation_during_enumeration() {
    let root = tempfile::tempdir().unwrap();
    for index in 0..600 {
        std::fs::write(root.path().join(format!("file-{index}.txt")), [0u8; 1]).unwrap();
    }
    let cancel = AtomicBool::new(false);
    let result = scan_dir_wide_local(&path_to_wide(root.path()), &mut Vec::new(), &cancel, |_| {
        cancel.store(true, Ordering::Relaxed);
    });

    assert!(cancel.load(Ordering::Relaxed));
    assert!(result.file_count > 0 && result.file_count < 600);
}

#[test]
fn recursive_progress_includes_pending_ancestors_without_double_counting() {
    let root = tempfile::tempdir().unwrap();
    let mut current = root.path().to_path_buf();
    for _ in 0..4 {
        std::fs::write(current.join("file.txt"), [0u8; 10]).unwrap();
        current = current.join("folder");
        std::fs::create_dir(&current).unwrap();
    }
    for index in 0..600 {
        std::fs::write(current.join(format!("file-{index}.txt")), [0u8; 1]).unwrap();
    }
    let progress = Mutex::new(Vec::new());
    let result = calculate_folder_size_parallel(
        root.path(),
        &Arc::new(AtomicBool::new(false)),
        |size| progress.lock().unwrap().push(size),
    )
    .unwrap();

    assert_eq!(result.total_size, 640);
    assert_eq!(result.file_count, 604);
    assert_eq!(result.folder_count, 4);
    let progress = progress.into_inner().unwrap();
    assert!(progress.iter().any(|&size| size > 40 && size < 640));
    assert!(progress.iter().all(|&size| size <= 640));
    assert_eq!(progress.last(), Some(&640));
}
