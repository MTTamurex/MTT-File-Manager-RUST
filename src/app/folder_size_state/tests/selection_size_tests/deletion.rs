use super::*;

#[test]
fn deletion_suspends_only_related_folders_and_resumes_after_last_operation() {
    let (mut state, requests) = state_with_worker();
    let items = vec![
        entry(r"C:\Data\Folder", true, 0),
        entry(r"C:\Other", true, 0),
        entry(r"C:\Data\file.txt", false, 12),
    ];
    let activity = state.delete_activity.clone();
    let first = activity.begin(&[PathBuf::from(r"C:\Data\Folder\Nested")]);
    let second = activity.begin(&[PathBuf::from(r"C:\Data\Folder\Other")]);
    let paths = selected(&items);
    let now = Instant::now();
    let summary = state.selection_size(&items, &paths, now);
    assert_eq!(summary.suspended_folders, 1);
    assert_eq!(summary.pending_folders, 1);
    assert_eq!(summary.total_size, 12);
    assert_eq!(
        requests.try_iter().map(|r| r.0).collect::<Vec<_>>(),
        vec![items[1].path.clone()]
    );
    drop(first);
    assert_eq!(
        state.selection_size(&items, &paths, now).suspended_folders,
        1
    );
    // Navigation cancels old requests, not the deletion's path-scoped guard.
    state.cancel_batch();
    assert!(state.delete_activity.affects(&items[0].path));
    drop(second);
    assert_eq!(
        state.selection_size(&items, &paths, now).suspended_folders,
        0
    );
    assert!(requests.try_iter().any(|r| r.0 == items[0].path));
}

#[test]
fn panel_and_batch_dispatch_do_not_start_affected_work() {
    let (mut state, requests) = state_with_worker();
    let activity = state.delete_activity.clone();
    let _deletion = activity.begin(&[PathBuf::from(r"C:\Data\Folder")]);
    for path in [r"C:\Data", r"C:\Data\Folder", r"C:\Data\Folder\Nested"] {
        let path = PathBuf::from(path);
        assert!(!state.enqueue_batch_size(path.clone()).unwrap());
        assert!(!state.dispatch_request(path.clone(), 0, Instant::now()));
        assert!(!state.request_panel_refresh(path, Instant::now()));
    }
    assert!(requests.try_recv().is_err());
    assert!(state
        .enqueue_batch_size(PathBuf::from(r"C:\Other"))
        .unwrap());
}

#[test]
fn suspended_batch_results_release_loading_without_publishing_or_failure() {
    let (mut state, requests) = state_with_worker();
    let path = PathBuf::from(r"C:\Data\Folder");
    assert!(state.enqueue_batch_size(path.clone()).unwrap());
    let (_, generation, epoch) = requests.recv().unwrap();
    let activity = state.delete_activity.clone();
    let deletion = activity.begin(std::slice::from_ref(&path));
    let result = BatchSizeResult {
        folder_path: path.clone(),
        summary: Some(FolderContentSummary::complete(100, 10, 2)),
        complete: true,
        cancelled: false,
        request_epoch: epoch,
        request_generation: generation,
    };
    assert!(!state.finish_batch_result(&result));
    assert!(!state.batch_loading.contains(&path));
    assert!(!state.batch_cache.contains(&path));
    assert!(!state.failed_until.contains_key(&path));
    drop(deletion);
    assert!(state.enqueue_batch_size(path).unwrap());
}

#[test]
fn suspended_sort_paths_do_not_consume_the_dispatch_budget() {
    let (mut state, requests) = state_with_worker();
    let activity = state.delete_activity.clone();
    let _deletion = activity.begin(&[PathBuf::from(r"C:\Data\Folder")]);
    let mut items: Vec<_> = (0..100)
        .map(|index| entry(&format!(r"C:\Data\Folder\Nested-{index}"), true, 0))
        .collect();
    items.push(entry(r"D:\Other", true, 0));
    state.prepare_size_sort(&items, Instant::now(), true);
    assert_eq!(
        requests.try_iter().map(|r| r.0).collect::<Vec<_>>(),
        vec![PathBuf::from(r"D:\Other")]
    );
}

#[test]
fn cancellation_arriving_after_deletion_ends_allows_immediate_retry() {
    let (mut state, requests) = state_with_worker();
    let path = PathBuf::from(r"C:\Data\Folder");
    state.enqueue_batch_size(path.clone()).unwrap();
    let (_, generation, epoch) = requests.recv().unwrap();
    let deletion = state
        .delete_activity
        .clone()
        .begin(std::slice::from_ref(&path));
    drop(deletion);
    assert!(!state.finish_batch_result(&BatchSizeResult {
        folder_path: path.clone(),
        summary: None,
        complete: true,
        cancelled: true,
        request_epoch: epoch,
        request_generation: generation,
    }));
    assert!(!state.batch_loading.contains(&path));
    assert!(!state.pending_revalidation.contains_key(&path));
    assert!(state.enqueue_batch_size(path).unwrap());
}
