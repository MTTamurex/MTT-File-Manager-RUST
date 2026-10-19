use super::*;

#[test]
fn selection_size_does_not_queue_virtual_archive_directories() {
    let (mut state, requests) = state_with_worker();
    let mut folder = entry("folder", true, 0);
    folder.path = PathBuf::from("archive.zip/folder");
    let items = vec![folder, entry("file.txt", false, 5)];
    let summary = state.selection_size(&items, &selected(&items), Instant::now());
    assert_eq!(summary.total_size, 5);
    assert_eq!(summary.pending_folders, 0);
    assert_eq!(summary.unavailable_items, 1);
    assert_eq!(requests.try_iter().count(), 0);
}

#[test]
fn selection_size_does_not_queue_virtual_view_paths() {
    let (mut state, requests) = state_with_worker();
    let items = vec![entry("::tag::1", true, 0), entry("file.txt", false, 5)];
    let summary = state.selection_size(&items, &selected(&items), Instant::now());
    assert_eq!(summary.total_size, 5);
    assert_eq!(summary.unavailable_items, 1);
    assert_eq!(summary.pending_folders, 0);
    assert_eq!(requests.try_iter().count(), 0);
}

#[test]
fn selection_size_marks_missing_entries_instead_of_reporting_a_complete_total() {
    let (mut state, _requests) = state_with_worker();
    let items = vec![
        entry("file-a.txt", false, 5),
        entry("file-b.txt", false, 10),
    ];
    let paths = selected(&items);
    let summary = state.selection_size(&items[..1], &paths, Instant::now());
    assert_eq!(summary.total_size, 5);
    assert_eq!(summary.unavailable_items, 1);
    // The details panel supplies the unfiltered listing, including hidden selections.
    let summary = state.selection_size(&items, &paths, Instant::now());
    assert_eq!(summary.total_size, 15);
    assert_eq!(summary.unavailable_items, 0);
}

#[test]
fn selection_size_counts_each_selected_path_only_once() {
    let (mut state, _requests) = state_with_worker();
    let items = vec![
        entry("file-a.txt", false, 5),
        entry("file-b.txt", false, 10),
        entry("file-a.txt", false, 5),
    ];
    let summary = state.selection_size(&items, &selected(&items), Instant::now());
    assert_eq!(summary.total_size, 15);
    assert_eq!(summary.unavailable_items, 0);
}

#[test]
fn selection_size_handles_worker_termination_after_accepting_requests() {
    let (mut state, requests) = state_with_worker();
    let items = vec![entry("folder-a", true, 0), entry("folder-b", true, 0)];
    let paths = selected(&items);
    let now = Instant::now();
    assert_eq!(state.selection_size(&items, &paths, now).pending_folders, 2);
    assert_eq!(requests.try_iter().count(), 2);
    state.cache_completed_folder_size(items[0].path.clone(), 20);
    assert!(state.fail_batch_requests());
    assert!(state.batch_loading.is_empty());
    assert!(state.batch_request_epochs.is_empty());
    let summary = state.selection_size(&items, &paths, now);
    assert_eq!(summary.total_size, 20);
    assert_eq!(summary.unavailable_items, 1);
    assert_eq!(summary.pending_folders, 0);
    assert_eq!(requests.try_iter().count(), 0);
    assert!(!state.fail_batch_requests());
}

#[test]
fn selection_size_still_accepts_independent_preview_completion_after_batch_worker_stops() {
    let (mut state, _requests) = state_with_worker();
    let items = vec![entry("folder-a", true, 0), entry("folder-b", true, 0)];
    let paths = selected(&items);
    let now = Instant::now();
    state.loading.insert(items[0].path.clone());
    state.batch_cache.put(items[0].path.clone(), 1);
    state.selection_size(&items, &paths, now);
    state.fail_batch_requests();
    let summary = state.selection_size(&items, &paths, now);
    assert_eq!(summary.total_size, 0);
    assert_eq!(summary.pending_folders, 1);
    assert_eq!(summary.unavailable_items, 1);
    state.loading.remove(&items[0].path);
    state.cache_completed_folder_size(items[0].path.clone(), 20);
    let summary = state.selection_size(&items, &paths, now);
    assert_eq!(summary.total_size, 20);
    assert_eq!(summary.pending_folders, 0);
    assert_eq!(summary.unavailable_items, 1);
}

#[test]
fn selection_size_does_not_use_partial_progress_from_a_failed_preview() {
    let (mut state, requests) = state_with_worker();
    let items = vec![entry("folder", true, 0), entry("file.txt", false, 5)];
    let now = Instant::now();
    state.batch_cache.put(items[0].path.clone(), 100);
    state.record_failure(items[0].path.clone(), now);
    let summary = state.selection_size(&items, &selected(&items), now);
    assert_eq!(summary.total_size, 5);
    assert_eq!(summary.unavailable_items, 1);
    assert_eq!(summary.pending_folders, 0);
    assert_eq!(requests.try_iter().count(), 0);
}

#[test]
fn selection_size_reuses_completed_sort_sizes_while_preview_is_still_loading() {
    let (mut state, requests) = state_with_worker();
    let items = vec![entry("folder", true, 0), entry("file.txt", false, 5)];
    state.size_sorting.sizes.insert(items[0].path.clone(), 20);
    state.loading.insert(items[0].path.clone());
    state.batch_cache.put(items[0].path.clone(), 1);
    let summary = state.selection_size(&items, &selected(&items), Instant::now());
    assert_eq!(summary.total_size, 25);
    assert_eq!(summary.unavailable_items, 0);
    assert_eq!(summary.pending_folders, 0);
    assert_eq!(requests.try_iter().count(), 0);
}
