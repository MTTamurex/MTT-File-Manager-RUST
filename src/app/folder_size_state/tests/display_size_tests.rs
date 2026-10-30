use super::*;

#[test]
fn confirmed_display_survives_invalidation_failure_navigation_and_partial_results() {
    let mut state = test_state();
    let path = PathBuf::from("folder");
    state.cache_completed_folder_size(path.clone(), 100);
    state.invalidate_calculated_folder_size(&path);
    state.batch_invalidation_epoch.insert(path.clone(), 1);
    assert!(state.batch_cache.peek(&path).is_none());
    assert!(state.export_size_snapshot().is_empty());
    state.batch_loading.insert(path.clone());
    state.batch_request_epochs.insert(path.clone(), 1);
    let mut result = BatchSizeResult {
        folder_path: path.clone(),
        total_size: Some(20),
        complete: false,
        request_epoch: 1,
        request_generation: 0,
    };
    assert!(!state.finish_batch_result(&result));
    assert_eq!(state.display_size_cache.peek(&path), Some(&100));
    result.complete = true;
    result.total_size = None;
    assert!(state.finish_batch_result(&result));
    state.record_failure(path.clone(), Instant::now());
    state.cancel_batch();
    assert_eq!(state.display_size_cache.peek(&path), Some(&100));
    assert!(state.export_size_snapshot().is_empty());
    // A valid result can shrink to zero; equal results are harmless too.
    for size in [100, 50, 0, 0] {
        state.cache_completed_folder_size(path.clone(), size);
        assert_eq!(state.display_size_cache.peek(&path), Some(&size));
    }
}

#[test]
fn display_cache_is_bounded_and_partial_values_never_enter_it() {
    let mut state = test_state();
    for n in 0..100 {
        let path = PathBuf::from(format!("folder-{n}"));
        state.batch_progress.insert(path.clone(), n);
        assert!(!state.display_size_cache.contains(&path));
        state.cache_completed_folder_size(path, n);
    }
    assert_eq!(state.display_size_cache.len(), 8);
}

#[test]
fn batch_dispatch_deduplicates_and_caps_all_producers() {
    let mut state = test_state();
    let (sender, receiver) = std::sync::mpsc::channel();
    state.batch_req_sender = sender;
    for n in 0..200 {
        let path = PathBuf::from(format!("folder-{n}"));
        state.enqueue_batch_size(path.clone()).unwrap();
        state.enqueue_batch_size(path).unwrap();
    }
    assert_eq!(state.batch_loading.len(), 120);
    assert_eq!(receiver.try_iter().count(), 120);
    // A terminal result frees capacity without requiring navigation.
    assert!(state.finish_batch_result(&BatchSizeResult {
        folder_path: PathBuf::from("folder-0"),
        total_size: Some(1),
        complete: true,
        request_epoch: 0,
        request_generation: 0,
    }));
    assert!(state
        .enqueue_batch_size(PathBuf::from("next-folder"))
        .unwrap());
    assert_eq!(receiver.try_iter().count(), 1);
    state.cancel_batch();
    let path = PathBuf::from("folder");
    state.enqueue_batch_size(path).unwrap();
    assert_eq!(receiver.try_iter().count(), 1);
}

#[test]
fn repeated_changes_keep_first_retry_deadline_without_dispatching_early() {
    let mut state = test_state();
    let (sender, receiver) = std::sync::mpsc::channel();
    state.batch_req_sender = sender;
    let path = PathBuf::from("folder");
    let now = Instant::now();
    for n in 0..30 {
        state.schedule_revalidation(path.clone(), now + Duration::from_millis(n * 100));
        assert!(!state.enqueue_batch_size(path.clone()).unwrap());
    }
    assert_eq!(receiver.try_iter().count(), 0);
    assert_eq!(
        state
            .take_expired_revalidations(now + Duration::from_secs(3))
            .len(),
        1
    );
    state.enqueue_batch_size(path).unwrap();
    assert_eq!(receiver.try_iter().count(), 1);
}

#[test]
fn stale_terminal_results_never_replace_last_confirmed_display() {
    let mut state = test_state();
    let path = PathBuf::from("folder");
    state.cache_completed_folder_size(path.clone(), 100);
    state.invalidate_calculated_folder_size(&path);
    state.batch_invalidation_epoch.insert(path.clone(), 2);
    state.batch_request_epochs.insert(path.clone(), 1);
    state.batch_loading.insert(path.clone());
    let mut result = BatchSizeResult {
        folder_path: path.clone(),
        total_size: Some(500),
        complete: true,
        request_epoch: 1,
        request_generation: 0,
    };
    assert!(!state.finish_batch_result(&result));
    assert_eq!(state.display_size_cache.peek(&path), Some(&100));
    state.cancel_batch();
    state.batch_loading.insert(path.clone());
    state.batch_request_epochs.insert(path.clone(), 2);
    result.request_epoch = 2;
    assert!(!state.finish_batch_result(&result));
    assert!(state.batch_loading.contains(&path));
    assert_eq!(state.display_size_cache.peek(&path), Some(&100));
}
