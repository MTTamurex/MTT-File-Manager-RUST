use super::*;
use crate::domain::file_entry::{FileEntry, SyncStatus};

fn folder(index: usize) -> FileEntry {
    let name = format!("folder-{index}");
    FileEntry {
        path: PathBuf::from(&name),
        name,
        is_dir: true,
        size: 0,
        modified: 0,
        created: None,
        folder_cover: None,
        drive_info: None,
        sync_status: SyncStatus::None,
        is_hidden: false,
        recycle_bin: None,
    }
}

#[test]
fn queues_offscreen_folders_in_bounded_batches_without_duplicates() {
    let mut state = test_state();
    let (sender, receiver) = std::sync::mpsc::channel();
    state.batch_req_sender = sender;
    let items: Vec<_> = (0..150).map(folder).collect();
    let now = Instant::now();
    state.prepare_size_sort(&items, now, true);
    assert_eq!(receiver.try_iter().count(), 30);
    state.prepare_size_sort(&items, now, false);
    assert_eq!(receiver.try_iter().count(), 0);
    for _ in 0..10 {
        state.prepare_size_sort(&items, now, true);
    }
    assert_eq!(receiver.try_iter().count(), 90);
    assert_eq!(state.batch_loading.len(), 120);
    let path = items
        .iter()
        .find(|item| state.batch_loading.contains(&item.path))
        .unwrap()
        .path
        .clone();
    state.batch_loading.remove(&path);
    state.size_sorting.complete(&path, 10);
    assert!(state.prepare_size_sort(&items, now, true));
    assert_eq!(receiver.try_iter().count(), 1);
}

#[test]
fn retains_completed_sizes_after_lru_eviction_and_recalculates_invalidated_sizes() {
    let mut state = test_state();
    let (sender, receiver) = std::sync::mpsc::channel();
    state.batch_req_sender = sender;
    let items: Vec<_> = (0..20).map(folder).collect();
    let now = Instant::now();
    state.prepare_size_sort(&items, now, true);
    for item in &items {
        state.batch_loading.remove(&item.path);
        state.batch_cache.put(item.path.clone(), 10);
        state.size_sorting.complete(&item.path, 10);
    }
    receiver.try_iter().for_each(drop);
    assert!(state.prepare_size_sort(&items, now, true));
    assert_eq!(state.size_sorting.sizes.len(), 20);
    assert!(!state.size_sorting.has_pending_sizes());
    assert_eq!(receiver.try_iter().count(), 0);
    let path = &items[0].path;
    state.batch_cache.pop(path);
    state.size_sorting.invalidate(path);
    state.batch_invalidation_epoch.insert(path.clone(), 2);
    assert!(state.prepare_size_sort(&items, now, true));
    let (requested_path, _, epoch) = receiver.try_recv().unwrap();
    assert_eq!(&requested_path, path);
    assert_eq!(epoch, 2);
    assert!(!state.size_sorting.sizes.contains_key(path));
    assert!(state.size_sorting.has_pending_sizes());
}

#[test]
fn reuses_valid_cached_sizes_without_queueing_work() {
    let mut state = test_state();
    let (sender, receiver) = std::sync::mpsc::channel();
    state.batch_req_sender = sender;
    let item = folder(0);
    state.batch_cache.put(item.path.clone(), 0);
    assert!(state.prepare_size_sort(std::slice::from_ref(&item), Instant::now(), true));
    assert_eq!(state.size_sorting.sizes.get(&item.path), Some(&0));
    assert!(!state.size_sorting.has_pending_sizes());
    assert_eq!(receiver.try_iter().count(), 0);
}

#[test]
fn final_completion_keeps_a_repaint_pending_until_the_batched_sort() {
    let mut state = test_state();
    let item = folder(0);
    let now = Instant::now();
    state.prepare_size_sort(std::slice::from_ref(&item), now, true);
    state.size_sorting.complete(&item.path, 10);
    assert!(!state.size_sorting.has_pending_sizes());
    assert!(state.size_sorting.needs_update());
    assert!(!state.prepare_size_sort(
        std::slice::from_ref(&item),
        now + Duration::from_millis(10),
        false
    ));
    assert!(state.size_sorting.needs_update());
    assert!(state.prepare_size_sort(&[item], now + Duration::from_millis(100), false));
    assert!(!state.size_sorting.needs_update());
}

#[test]
fn navigation_clears_pinned_sizes_and_ignores_untracked_completions() {
    let mut state = test_state();
    let item = folder(0);
    state.prepare_size_sort(std::slice::from_ref(&item), Instant::now(), true);
    state.size_sorting.complete(&item.path, 10);
    state.cancel_batch();
    state.size_sorting.complete(&item.path, 20);
    assert!(state.size_sorting.sizes.is_empty());
}

#[test]
fn preview_progress_is_not_used_as_a_completed_size() {
    let mut state = test_state();
    let item = folder(0);
    state.loading.insert(item.path.clone());
    state.batch_cache.put(item.path.clone(), 10);
    state.prepare_size_sort(std::slice::from_ref(&item), Instant::now(), true);
    assert!(!state.size_sorting.sizes.contains_key(&item.path));
}

#[test]
fn both_visible_panels_keep_their_completed_sizes() {
    let mut state = test_state();
    let left = vec![folder(0)];
    let right = vec![folder(1)];
    let now = Instant::now();
    state.prepare_size_sort_for_listings(&left, &right, now, true);
    state.size_sorting.complete(&left[0].path, 10);
    state.size_sorting.complete(&right[0].path, 20);
    assert!(state.prepare_size_sort_for_listings(&left, &right, now, true));
    assert_eq!(state.size_sorting.sizes.len(), 2);
    assert!(!state.size_sorting.has_pending_sizes());
    // Changing panel focus keeps the union instead of evicting the other pane.
    state.prepare_size_sort_for_listings(&right, &left, now, true);
    assert_eq!(state.size_sorting.sizes.len(), 2);
}

#[test]
fn asynchronous_listing_replacement_is_not_marked_as_sorted() {
    let mut state = test_state();
    let original = Arc::new(vec![folder(0), folder(1)]);
    let other_panel = Arc::new(vec![folder(2)]);
    state.size_sorting.remember_sorted(&original);
    state.size_sorting.remember_sorted(&other_panel);
    assert!(state.size_sorting.is_sorted(&original));
    assert!(state.size_sorting.is_sorted(&other_panel));
    let replacement = Arc::new(original.as_ref().clone());
    assert!(!state.size_sorting.is_sorted(&replacement));
    state.size_sorting.remember_sorted(&replacement);
    assert!(state.size_sorting.is_sorted(&replacement));
    assert!(state.size_sorting.is_sorted(&other_panel));
    state.size_sorting.forget_sorted_listings();
    assert!(!state.size_sorting.is_sorted(&replacement));
    assert!(!state.size_sorting.is_sorted(&other_panel));
}

#[test]
fn disconnected_worker_does_not_keep_requesting_repaints() {
    let mut state = test_state();
    state.prepare_size_sort(&[folder(0)], Instant::now(), true);
    assert!(state.size_sorting.has_pending_sizes());
    assert!(!state.size_sorting.needs_update());
    assert!(state.batch_loading.is_empty());
}

#[test]
fn old_navigation_result_cannot_release_a_new_request_for_the_same_folder() {
    let mut state = test_state();
    let path = folder(0).path;
    let generation = state.batch_generation.load(Ordering::Acquire);
    state.cancel_batch();
    state.batch_loading.insert(path.clone());
    let mut result = BatchSizeResult {
        folder_path: path.clone(),
        total_size: Some(10),
        request_epoch: 0,
        request_generation: generation,
    };
    assert!(!state.finish_batch_result(&result));
    assert!(state.batch_loading.contains(&path));
    result.request_generation = state.batch_generation.load(Ordering::Acquire);
    assert!(state.finish_batch_result(&result));
    assert!(!state.batch_loading.contains(&path));
}

#[test]
fn invalidated_batch_result_is_rejected_and_releases_its_loading_slot() {
    let mut state = test_state();
    let path = folder(0).path;
    state.batch_loading.insert(path.clone());
    state.batch_invalidation_epoch.insert(path.clone(), 2);
    let result = BatchSizeResult {
        folder_path: path.clone(),
        total_size: Some(10),
        request_epoch: 1,
        request_generation: state.batch_generation.load(Ordering::Acquire),
    };
    assert!(!state.finish_batch_result(&result));
    assert!(!state.batch_loading.contains(&path));
}

#[test]
fn old_epoch_result_does_not_release_a_newer_retry() {
    let mut state = test_state();
    let (sender, _receiver) = std::sync::mpsc::channel();
    state.batch_req_sender = sender;
    let path = folder(0).path;
    state.enqueue_batch_size(path.clone()).unwrap();
    state.batch_invalidation_epoch.insert(path.clone(), 1);
    // Deferred revalidation releases a failed request before sending its retry.
    state.batch_loading.remove(&path);
    state.enqueue_batch_size(path.clone()).unwrap();
    let mut result = BatchSizeResult {
        folder_path: path.clone(),
        total_size: Some(10),
        request_epoch: 0,
        request_generation: state.batch_generation.load(Ordering::Acquire),
    };
    assert!(!state.finish_batch_result(&result));
    assert!(state.batch_loading.contains(&path));
    assert_eq!(state.batch_request_epochs.get(&path), Some(&1));
    result.request_epoch = 1;
    assert!(state.finish_batch_result(&result));
    assert!(!state.batch_loading.contains(&path));
    assert!(!state.batch_request_epochs.contains_key(&path));
}
