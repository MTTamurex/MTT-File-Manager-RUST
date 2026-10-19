use super::*;
use crate::domain::file_entry::{FileEntry, SyncStatus};

mod review;

fn entry(name: &str, is_dir: bool, size: u64) -> FileEntry {
    FileEntry {
        path: PathBuf::from(name),
        name: name.into(),
        is_dir,
        size,
        modified: 0,
        created: None,
        folder_cover: None,
        drive_info: None,
        sync_status: SyncStatus::None,
        is_hidden: false,
        recycle_bin: None,
    }
}

fn selected(items: &[FileEntry]) -> FxHashSet<PathBuf> {
    items.iter().map(|item| item.path.clone()).collect()
}

fn state_with_worker() -> (FolderSizeState, std::sync::mpsc::Receiver<BatchSizeRequest>) {
    let mut state = test_state();
    let (sender, receiver) = std::sync::mpsc::channel();
    state.batch_req_sender = sender;
    (state, receiver)
}

#[test]
fn selection_size_sums_folders_in_background_without_size_sorting() {
    let (mut state, requests) = state_with_worker();
    let items = vec![entry("folder-a", true, 0), entry("folder-b", true, 0)];
    let paths = selected(&items);
    let now = Instant::now();
    let summary = state.selection_size(&items, &paths, now);
    assert_eq!(summary.total_size, 0);
    assert_eq!(summary.pending_folders, 2);
    assert_eq!(requests.try_iter().count(), 2);
    state.cache_completed_folder_size(items[0].path.clone(), 100);
    state.batch_loading.remove(&items[0].path);
    let summary = state.selection_size(&items, &paths, now);
    assert_eq!(summary.total_size, 100);
    assert_eq!(summary.pending_folders, 1);
    state.cache_completed_folder_size(items[1].path.clone(), 250);
    state.batch_loading.remove(&items[1].path);
    let summary = state.selection_size(&items, &paths, now);
    assert_eq!(summary.total_size, 350);
    assert_eq!(summary.pending_folders, 0);
    assert_eq!(summary.unavailable_items, 0);
    assert_eq!(requests.try_iter().count(), 0);
    assert!(state.size_sorting.sizes.is_empty());
}

#[test]
fn selection_size_keeps_file_totals_and_includes_folders_and_archives() {
    let (mut state, requests) = state_with_worker();
    let items = vec![
        entry("file.txt", false, 20),
        entry("folder", true, 0),
        entry("archive.zip", true, 30),
    ];
    state.batch_cache.put(items[1].path.clone(), 100);
    let summary = state.selection_size(&items, &selected(&items), Instant::now());
    assert_eq!(summary.total_size, 150);
    assert_eq!(summary.pending_folders, 0);
    assert_eq!(requests.try_iter().count(), 0);
}

#[test]
fn selection_size_distinguishes_empty_folders_from_pending_folders() {
    let (mut state, _requests) = state_with_worker();
    let items = vec![entry("empty", true, 0), entry("pending", true, 0)];
    state.batch_cache.put(items[0].path.clone(), 0);
    let summary = state.selection_size(&items, &selected(&items), Instant::now());
    assert_eq!(summary.total_size, 0);
    assert_eq!(summary.pending_folders, 1);
}

#[test]
fn selection_size_reuses_a_request_and_rejects_preview_partial_totals() {
    let (mut state, requests) = state_with_worker();
    let items = vec![entry("folder-a", true, 0), entry("folder-b", true, 0)];
    state.loading.insert(items[0].path.clone());
    state.batch_cache.put(items[0].path.clone(), 100);
    state.enqueue_batch_size(items[1].path.clone()).unwrap();
    requests.try_iter().for_each(drop);
    let summary = state.selection_size(&items, &selected(&items), Instant::now());
    assert_eq!(summary.total_size, 0);
    assert_eq!(summary.pending_folders, 2);
    assert_eq!(requests.try_iter().count(), 0);
}

#[test]
fn selection_size_survives_cache_eviction() {
    let (mut state, _requests) = state_with_worker();
    let items: Vec<_> = (0..20)
        .map(|index| entry(&format!("folder-{index}"), true, 0))
        .collect();
    let paths = selected(&items);
    let now = Instant::now();
    state.selection_size(&items, &paths, now);
    for (index, item) in items.iter().enumerate() {
        state.cache_completed_folder_size(item.path.clone(), index as u64 + 1);
    }
    let summary = state.selection_size(&items, &paths, now);
    assert_eq!(summary.total_size, 210);
    assert_eq!(summary.pending_folders, 0);
}

#[test]
fn selection_size_updates_when_the_selection_changes_or_a_folder_is_invalidated() {
    let (mut state, requests) = state_with_worker();
    let items = vec![
        entry("folder-a", true, 0),
        entry("folder-b", true, 0),
        entry("folder-c", true, 0),
    ];
    let now = Instant::now();
    for item in &items {
        state.batch_cache.put(item.path.clone(), 10);
    }
    assert_eq!(
        state
            .selection_size(&items, &selected(&items[..2]), now)
            .total_size,
        20
    );
    let paths = selected(&items[1..]);
    assert_eq!(state.selection_size(&items, &paths, now).total_size, 20);
    state.invalidate_calculated_folder_size(&items[1].path);
    let summary = state.selection_size(&items, &paths, now);
    assert_eq!(summary.total_size, 10);
    assert_eq!(summary.pending_folders, 1);
    assert_eq!(requests.try_iter().count(), 1);
    state.cache_completed_folder_size(items[0].path.clone(), 500);
    assert_eq!(state.selection_size(&items, &paths, now).total_size, 10);
    state.cache_completed_folder_size(items[1].path.clone(), 30);
    assert_eq!(state.selection_size(&items, &paths, now).total_size, 40);
}

#[test]
fn selection_size_reports_unavailable_and_accepts_a_successful_retry() {
    let (mut state, _requests) = state_with_worker();
    let items = vec![entry("folder", true, 0), entry("file.txt", false, 5)];
    let paths = selected(&items);
    let now = Instant::now();
    state.selection_size(&items, &paths, now);
    state.selection_size.failed(&items[0].path);
    let summary = state.selection_size(&items, &paths, now);
    assert_eq!(summary.total_size, 5);
    assert_eq!(summary.unavailable_items, 1);
    assert_eq!(summary.pending_folders, 0);
    state.cache_completed_folder_size(items[0].path.clone(), 100);
    let summary = state.selection_size(&items, &paths, now);
    assert_eq!(summary.total_size, 105);
    assert_eq!(summary.unavailable_items, 0);
}

#[test]
fn selection_size_handles_a_stopped_worker_without_a_permanent_loading_state() {
    let mut state = test_state();
    let items = vec![entry("folder", true, 0), entry("file.txt", false, 5)];
    let summary = state.selection_size(&items, &selected(&items), Instant::now());
    assert_eq!(summary.total_size, 5);
    assert_eq!(summary.unavailable_items, 1);
    assert_eq!(summary.pending_folders, 0);
    assert!(state.batch_loading.is_empty());
}

#[test]
fn selection_size_bounds_work_and_clears_state_for_single_selection() {
    let (mut state, requests) = state_with_worker();
    let items: Vec<_> = (0..150)
        .map(|index| entry(&format!("folder-{index}"), true, 0))
        .collect();
    let paths = selected(&items);
    let now = Instant::now();
    state.selection_size(&items, &paths, now);
    assert_eq!(requests.try_iter().count(), 30);
    for _ in 0..10 {
        state.selection_size(&items, &paths, now);
    }
    assert_eq!(requests.try_iter().count(), 90);
    assert_eq!(state.batch_loading.len(), 120);
    assert_eq!(
        state.selection_size(&items, &selected(&items[..1]), now),
        SelectionSizeSummary::default()
    );
    state.cancel_batch();
    assert!(state.batch_loading.is_empty());
}

#[test]
fn selection_size_saturates_instead_of_overflowing() {
    let (mut state, _requests) = state_with_worker();
    let items = vec![
        entry("file-a.txt", false, u64::MAX),
        entry("file-b.txt", false, 1),
    ];
    assert_eq!(
        state
            .selection_size(&items, &selected(&items), Instant::now())
            .total_size,
        u64::MAX
    );
}
