use super::*;
use crate::domain::file_entry::FileEntry;

fn pending_scan() -> (FolderSizeState, BatchSizeResult) {
    let mut state = test_state();
    let path = PathBuf::from("folder");
    state.batch_loading.insert(path.clone());
    state.batch_request_epochs.insert(path.clone(), 0);
    let result = BatchSizeResult {
        folder_path: path,
        total_size: Some(10),
        complete: false,
        request_epoch: 0,
        request_generation: 0,
    };
    (state, result)
}

#[test]
fn partial_sizes_only_update_display_and_keep_request_pending() {
    let (mut state, mut result) = pending_scan();
    let path = result.folder_path.clone();
    assert!(!state.finish_batch_result(&result));
    assert_eq!(state.batch_progress.get(&path), Some(&10));
    assert!(state.batch_loading.contains(&path));
    assert_eq!(state.batch_request_epochs.get(&path), Some(&0));
    assert!(state.batch_cache.peek(&path).is_none());
    assert!(state.cache.peek(&path).is_none());

    result.total_size = Some(5);
    assert!(!state.finish_batch_result(&result));
    assert_eq!(state.batch_progress.get(&path), Some(&10));
    result.total_size = Some(30);
    assert!(!state.finish_batch_result(&result));
    assert_eq!(state.batch_progress.get(&path), Some(&30));

    result.complete = true;
    assert!(state.finish_batch_result(&result));
    assert!(!state.batch_progress.contains_key(&path));
    assert!(!state.batch_loading.contains(&path));
    state.cache_completed_folder_size(path.clone(), 30);
    assert_eq!(state.batch_cache.peek(&path), Some(&30));
    result.complete = false;
    assert!(!state.finish_batch_result(&result));
    assert!(!state.batch_progress.contains_key(&path));
}

#[test]
fn partial_sizes_do_not_complete_sorting_or_selection() {
    let (mut state, result) = pending_scan();
    assert!(!state.finish_batch_result(&result));
    let items = vec![
        FileEntry {
            name: "folder".into(),
            path: result.folder_path.clone(),
            is_dir: true,
            size: 0,
            modified: 0,
            created: None,
            folder_cover: None,
            drive_info: None,
            sync_status: crate::domain::file_entry::SyncStatus::None,
            is_hidden: false,
            recycle_bin: None,
        },
        FileEntry {
            name: "file.txt".into(),
            path: PathBuf::from("file.txt"),
            size: 7,
            is_dir: false,
            modified: 0,
            created: None,
            folder_cover: None,
            drive_info: None,
            sync_status: crate::domain::file_entry::SyncStatus::None,
            is_hidden: false,
            recycle_bin: None,
        },
    ];
    let selected = items.iter().map(|item| item.path.clone()).collect();
    state.prepare_size_sort(&items, Instant::now(), true);
    assert!(!state.size_sorting.sizes.contains_key(&result.folder_path));
    let summary = state.selection_size(&items, &selected, Instant::now());
    assert_eq!(summary.total_size, 7);
    assert_eq!(summary.pending_folders, 1);
}

#[test]
fn navigation_discards_partial_sizes_and_rejects_old_progress() {
    let (mut state, mut result) = pending_scan();
    state.finish_batch_result(&result);
    state.cancel_batch();
    assert!(state.batch_progress.is_empty());
    state.batch_loading.insert(result.folder_path.clone());
    state
        .batch_request_epochs
        .insert(result.folder_path.clone(), 0);
    assert!(!state.finish_batch_result(&result));
    assert!(state.batch_progress.is_empty());
    assert!(state.batch_loading.contains(&result.folder_path));
    result.complete = true;
    assert!(!state.finish_batch_result(&result));
    assert!(state.batch_loading.contains(&result.folder_path));
}

#[test]
fn invalidation_rejects_old_progress_without_releasing_request() {
    let (mut state, mut result) = pending_scan();
    state.finish_batch_result(&result);
    state.invalidate_calculated_folder_size(&result.folder_path);
    state
        .batch_invalidation_epoch
        .insert(result.folder_path.clone(), 1);
    assert!(!state.finish_batch_result(&result));
    assert!(state.batch_progress.is_empty());
    assert!(state.batch_loading.contains(&result.folder_path));
    result.complete = true;
    assert!(!state.finish_batch_result(&result));
    assert!(!state.batch_loading.contains(&result.folder_path));
}

#[test]
fn failure_and_completed_preview_clear_partial_sizes() {
    let (mut state, mut result) = pending_scan();
    state.finish_batch_result(&result);
    result.complete = true;
    result.total_size = None;
    assert!(state.finish_batch_result(&result));
    assert!(state.batch_progress.is_empty());

    let (mut state, result) = pending_scan();
    state.finish_batch_result(&result);
    state.cache_completed_folder_size(result.folder_path.clone(), 50);
    assert!(state.batch_progress.is_empty());

    let (mut state, result) = pending_scan();
    state.finish_batch_result(&result);
    state.fail_batch_requests();
    assert!(state.batch_progress.is_empty());
}

#[test]
fn preview_owns_display_progress_while_its_request_is_active() {
    let (mut state, mut result) = pending_scan();
    state.finish_batch_result(&result);
    assert_eq!(state.batch_progress.get(&result.folder_path), Some(&10));
    let (sender, _receiver) = std::sync::mpsc::channel();
    state.req_sender = sender;
    assert!(state.dispatch_request(result.folder_path.clone(), 0, Instant::now()));
    assert!(state.batch_progress.is_empty());

    result.total_size = Some(20);
    assert!(!state.finish_batch_result(&result));
    assert!(state.batch_progress.is_empty());
    assert!(state.batch_loading.contains(&result.folder_path));

    // Cancelling the preview allows the independent batch scan to resume display updates.
    state.cancel_active_request(&result.folder_path);
    assert!(!state.finish_batch_result(&result));
    assert_eq!(state.batch_progress.get(&result.folder_path), Some(&20));
}

#[test]
fn old_retry_progress_cannot_overwrite_or_release_a_newer_scan() {
    let (mut state, mut result) = pending_scan();
    state
        .batch_request_epochs
        .insert(result.folder_path.clone(), 1);
    state
        .batch_invalidation_epoch
        .insert(result.folder_path.clone(), 1);
    state.batch_progress.insert(result.folder_path.clone(), 50);
    assert!(!state.finish_batch_result(&result));
    assert_eq!(state.batch_progress.get(&result.folder_path), Some(&50));
    result.complete = true;
    assert!(!state.finish_batch_result(&result));
    assert_eq!(state.batch_progress.get(&result.folder_path), Some(&50));
    assert!(state.batch_loading.contains(&result.folder_path));
    assert_eq!(
        state.batch_request_epochs.get(&result.folder_path),
        Some(&1)
    );
}
