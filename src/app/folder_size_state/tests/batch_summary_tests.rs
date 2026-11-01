use super::*;

fn pending_summary() -> (FolderSizeState, BatchSizeResult) {
    let mut state = test_state();
    let path = PathBuf::from("folder");
    state.batch_loading.insert(path.clone());
    state.batch_request_epochs.insert(path.clone(), 0);
    let result = BatchSizeResult {
        folder_path: path,
        summary: Some(FolderContentSummary::complete(100, 12, 3)),
        complete: true,
        request_epoch: 0,
        request_generation: 0,
    };
    (state, result)
}

#[test]
fn completed_batch_reuses_the_same_triplet_in_list_panel_and_export() {
    let (mut state, result) = pending_summary();
    let path = &result.folder_path;
    assert!(state.finish_batch_result(&result));
    assert_eq!(state.batch_cache.peek(path), Some(&100));
    assert_eq!(state.cache.peek(path).copied(), result.summary);
    let (summary, loading) = state.summary_for_panel_render(path);
    assert_eq!(summary, result.summary);
    assert!(!loading);
    // The panel requests its first calculation only for absent/incomplete counts.
    assert!(summary.unwrap().has_counts());
    assert_eq!(state.export_size_snapshot().get("folder"), Some(&100));
    assert!(state.panel_deferred_revalidation.is_empty());
    assert!(state.active_requests.is_empty());
}

#[test]
fn fresh_batch_replaces_all_old_counts_including_a_legitimate_empty_result() {
    let (mut state, mut result) = pending_summary();
    assert!(state.finish_batch_result(&result));
    for summary in [
        FolderContentSummary::complete(50, 2, 1),
        FolderContentSummary::complete(0, 0, 0),
    ] {
        state.batch_loading.insert(result.folder_path.clone());
        state
            .batch_request_epochs
            .insert(result.folder_path.clone(), 0);
        result.summary = Some(summary);
        assert!(state.finish_batch_result(&result));
        assert_eq!(state.cache.peek(&result.folder_path), Some(&summary));
        assert_eq!(
            state.summary_for_panel_render(&result.folder_path),
            (Some(summary), false)
        );
        assert_eq!(
            state.batch_cache.peek(&result.folder_path),
            Some(&summary.total_size)
        );
    }
}

#[test]
fn progress_and_failure_never_publish_a_confirmed_triplet() {
    let (mut state, mut result) = pending_summary();
    result.complete = false;
    assert!(!state.finish_batch_result(&result));
    assert!(state.cache.is_empty());
    assert!(state.panel_stale_cache.is_empty());
    assert!(state.batch_cache.is_empty());
    result.complete = true;
    result.summary = None;
    assert!(state.finish_batch_result(&result));
    assert!(state.cache.is_empty());
    assert!(state.panel_stale_cache.is_empty());
    assert!(state.batch_cache.is_empty());
}

#[test]
fn size_only_completion_does_not_inherit_old_counts() {
    let (mut state, mut result) = pending_summary();
    state
        .cache
        .put(result.folder_path.clone(), result.summary.unwrap());
    let size_only = FolderContentSummary::size_only(50);
    result.summary = Some(size_only);
    assert!(state.finish_batch_result(&result));
    assert_eq!(state.cache.peek(&result.folder_path), Some(&size_only));
    assert!(!state.cache.peek(&result.folder_path).unwrap().has_counts());
    assert!(state.panel_stale_cache.is_empty());
}

#[test]
fn failed_batch_does_not_turn_a_previous_summary_into_zero() {
    let (mut state, mut result) = pending_summary();
    assert!(state.finish_batch_result(&result));
    let confirmed = result.summary;
    state.batch_loading.insert(result.folder_path.clone());
    state
        .batch_request_epochs
        .insert(result.folder_path.clone(), 0);
    result.summary = None;
    assert!(state.finish_batch_result(&result));
    assert_eq!(state.cache.peek(&result.folder_path).copied(), confirmed);
    assert_eq!(
        state.summary_for_panel_render(&result.folder_path),
        (confirmed, false)
    );
    assert_eq!(state.batch_cache.peek(&result.folder_path), Some(&100));
}

#[test]
fn reused_summaries_and_panel_retry_bookkeeping_remain_bounded() {
    let (mut state, mut result) = pending_summary();
    for n in 0..100 {
        result.folder_path = PathBuf::from(format!("folder-{n}"));
        state.batch_loading.insert(result.folder_path.clone());
        state
            .batch_request_epochs
            .insert(result.folder_path.clone(), 0);
        assert!(state.finish_batch_result(&result));
        state.reschedule_panel_revalidation_if_stale(&result.folder_path, Instant::now());
    }
    assert_eq!(state.cache.len(), 8);
    assert_eq!(state.batch_cache.len(), 8);
    assert_eq!(state.panel_stale_cache.len(), 8);
    assert_eq!(state.panel_deferred_revalidation.len(), 8);
}

#[test]
fn invalidation_and_navigation_reject_the_entire_batch_snapshot() {
    for navigation in [false, true] {
        let (mut state, result) = pending_summary();
        if navigation {
            state.cancel_batch();
        } else {
            state
                .batch_invalidation_epoch
                .insert(result.folder_path.clone(), 1);
        }
        assert!(!state.finish_batch_result(&result));
        assert!(state.cache.is_empty());
        assert!(state.panel_stale_cache.is_empty());
        assert!(state.batch_cache.is_empty());
    }
}

#[test]
fn old_batch_cannot_replace_a_more_recent_panel_completion() {
    let (mut state, result) = pending_summary();
    let path = &result.folder_path;
    let newer = FolderContentSummary::complete(50, 2, 1);
    state.cache.put(path.clone(), newer);
    state.cache_completed_panel_summary(path.clone(), newer);
    state.cache_completed_folder_size(path.clone(), newer.total_size);
    // Panel completion advances the epoch before the batch queue is drained.
    state.batch_invalidation_epoch.insert(path.clone(), 1);
    assert!(!state.finish_batch_result(&result));
    assert_eq!(state.cache.peek(path), Some(&newer));
    assert_eq!(state.summary_for_panel_render(path), (Some(newer), false));
    assert_eq!(state.batch_cache.peek(path), Some(&50));
}

#[test]
fn old_batch_cannot_release_a_retry_or_overwrite_its_confirmed_counts() {
    let (mut state, result) = pending_summary();
    let path = &result.folder_path;
    let newer = FolderContentSummary::complete(50, 2, 1);
    state.cache.put(path.clone(), newer);
    state.cache_completed_panel_summary(path.clone(), newer);
    state.cache_completed_folder_size(path.clone(), newer.total_size);
    state.batch_invalidation_epoch.insert(path.clone(), 1);
    state.batch_request_epochs.insert(path.clone(), 1);
    assert!(!state.finish_batch_result(&result));
    assert!(state.batch_loading.contains(path));
    assert_eq!(state.batch_request_epochs.get(path), Some(&1));
    assert_eq!(state.cache.peek(path), Some(&newer));
    assert_eq!(state.summary_for_panel_render(path), (Some(newer), false));
    assert_eq!(state.batch_cache.peek(path), Some(&50));
}

#[test]
fn batch_reuse_does_not_cancel_the_usn_settling_revalidation() {
    let (mut state, result) = pending_summary();
    let path = &result.folder_path;
    let now = Instant::now();
    state.schedule_revalidation(path.clone(), now);
    assert!(state.finish_batch_result(&result));
    assert!(state.pending_revalidation.contains_key(path));
    assert!(state.export_size_snapshot().is_empty());
    let due = now + Duration::from_secs(3);
    assert_eq!(
        state.take_expired_revalidations(due),
        vec![(path.clone(), false)]
    );
    // Mirror the deferred invalidation: retain the display, schedule a fresh
    // panel query, and invalidate both usable caches and in-flight responses.
    state.reschedule_panel_revalidation_if_stale(path, due);
    state.invalidate_calculated_folder_size(path);
    state.cache.pop(path);
    state.batch_invalidation_epoch.insert(path.clone(), 1);
    assert!(state.take_due_panel_revalidation(due, path).is_none());
    assert_eq!(
        state.take_due_panel_revalidation(due + PANEL_STALE_REVALIDATION_DELAY, path),
        Some(path.clone())
    );
    assert!(!state.finish_batch_result(&result));
    assert!(state.cache.is_empty());
    assert!(state.batch_cache.is_empty());
}
