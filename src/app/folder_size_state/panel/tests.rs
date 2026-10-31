use super::*;
use crate::app::folder_size_state::tests::test_state;

fn connected_state() -> (
    FolderSizeState,
    std::sync::mpsc::Receiver<FolderSizeRequest>,
) {
    let mut state = test_state();
    let (sender, receiver) = std::sync::mpsc::channel();
    state.req_sender = sender;
    (state, receiver)
}

#[test]
fn confirmed_triplet_survives_progress_size_only_results_and_failures() {
    let mut state = test_state();
    let path = PathBuf::from("folder");
    let confirmed = FolderContentSummary::complete(100, 12, 3);
    state.cache_completed_panel_summary(path.clone(), confirmed);
    state.loading.insert(path.clone());
    for progress in [
        FolderContentSummary::size_only(5),
        FolderContentSummary::complete(50, 2, 1),
    ] {
        state.cache.put(path.clone(), progress);
        assert!(state.loading.contains(&path));
        assert!(state.export_size_snapshot().is_empty());
        assert_eq!(
            state.summary_for_panel_render(&path),
            (Some(confirmed), false)
        );
    }
    state.cache.pop(&path);
    state.record_failure(path.clone(), Instant::now());
    assert_eq!(
        state.summary_for_panel_render(&path),
        (Some(confirmed), false)
    );
    for size in [100, 50, 0, 0] {
        let next = FolderContentSummary::complete(size, 1, 0);
        state.cache_completed_panel_summary(path.clone(), next);
        assert_eq!(state.summary_for_panel_render(&path), (Some(next), false));
    }
}

#[test]
fn first_calculation_still_exposes_loading_and_never_promotes_progress() {
    let mut state = test_state();
    let path = PathBuf::from("folder");
    state.loading.insert(path.clone());
    let partial = FolderContentSummary::complete(10, 1, 1);
    state.cache.put(path.clone(), partial);
    state.reschedule_panel_revalidation_if_stale(&path, Instant::now());
    assert!(state.panel_stale_cache.is_empty());
    assert_eq!(state.summary_for_panel_render(&path), (Some(partial), true));
}

#[test]
fn continuous_events_keep_first_deadline_and_do_not_duplicate_or_cancel_active_work() {
    let (mut state, receiver) = connected_state();
    let path = PathBuf::from("folder");
    let now = Instant::now();
    state.cache_completed_panel_summary(path.clone(), FolderContentSummary::complete(10, 1, 0));
    for n in 0..30 {
        let time = now + Duration::from_millis(n * 100);
        state.reschedule_panel_revalidation_if_stale(&path, time);
        state.schedule_revalidation(path.clone(), time);
        assert!(!state.request_panel_refresh(path.clone(), time));
    }
    assert!(receiver.try_recv().is_err());
    let due = now + PANEL_STALE_REVALIDATION_DELAY;
    assert!(state.take_due_panel_revalidation(due, &path).is_none());
    state.take_expired_revalidations(due);
    assert_eq!(
        state.take_due_panel_revalidation(due, &path),
        Some(path.clone())
    );
    assert!(state.request_panel_refresh(path.clone(), due));
    let request = receiver.try_recv().unwrap();
    state.cancel.store(false, Ordering::Release);
    for n in 1..30 {
        let time = due + Duration::from_millis(n * 100);
        state.invalidate_calculated_folder_size(&path);
        *state
            .batch_invalidation_epoch
            .entry(path.clone())
            .or_default() += 1;
        state.reschedule_panel_revalidation_if_stale(&path, time);
        assert!(!state.request_panel_refresh(path.clone(), time));
        assert!(state.is_active_request(&path, request.request_id));
        assert!(!state.cancel.load(Ordering::Acquire));
    }
    assert!(receiver.try_recv().is_err());
}

#[test]
fn failure_cooldown_applies_even_when_events_keep_arriving() {
    let (mut state, receiver) = connected_state();
    let path = PathBuf::from("folder");
    let now = Instant::now();
    state.cache_completed_panel_summary(path.clone(), FolderContentSummary::complete(10, 1, 0));
    state.record_failure(path.clone(), now);
    for second in 1..30 {
        let time = now + Duration::from_secs(second);
        state.reschedule_panel_revalidation_if_stale(&path, time);
        assert!(state.take_due_panel_revalidation(time, &path).is_none());
        assert!(!state.request_panel_refresh(path.clone(), time));
    }
    assert!(receiver.try_recv().is_err());
    let retry = now + FOLDER_SIZE_FAILURE_RETRY_DELAY;
    assert_eq!(
        state.take_due_panel_revalidation(retry, &path),
        Some(path.clone())
    );
    assert!(state.request_panel_refresh(path, retry));
    assert_eq!(receiver.try_iter().count(), 1);
}

#[test]
fn navigation_releases_cancelled_work_but_retains_confirmed_display_and_retry() {
    let (mut state, receiver) = connected_state();
    let path = PathBuf::from("folder");
    let confirmed = FolderContentSummary::complete(10, 1, 0);
    state.cache_completed_panel_summary(path.clone(), confirmed);
    assert!(state.request_panel_refresh(path.clone(), Instant::now()));
    let old = receiver.try_recv().unwrap();
    state
        .cache
        .put(path.clone(), FolderContentSummary::size_only(2));
    state.batch_cache.put(path.clone(), 2);
    state.cancel_batch();
    assert!(!state.loading.contains(&path));
    assert!(!state.is_active_request(&path, old.request_id));
    assert!(state.cache.peek(&path).is_none());
    assert!(state.batch_cache.peek(&path).is_none());
    assert_eq!(
        state.summary_for_panel_render(&path),
        (Some(confirmed), false)
    );
    assert!(state.panel_deferred_revalidation.contains_key(&path));
}

#[test]
fn completed_summary_and_retry_bookkeeping_are_bounded_together() {
    let mut state = test_state();
    for n in 0..100 {
        let path = PathBuf::from(format!("folder-{n}"));
        state.cache_completed_panel_summary(path.clone(), FolderContentSummary::complete(n, 1, 0));
        state.reschedule_panel_revalidation_if_stale(&path, Instant::now());
    }
    assert_eq!(state.panel_stale_cache.len(), 8);
    assert_eq!(state.panel_deferred_revalidation.len(), 8);
}

#[test]
fn stalled_refresh_times_out_and_backs_off_without_losing_the_confirmed_summary() {
    let (mut state, receiver) = connected_state();
    let path = PathBuf::from("folder");
    let now = Instant::now();
    let confirmed = FolderContentSummary::complete(10, 1, 0);
    state.cache_completed_panel_summary(path.clone(), confirmed);
    assert!(state.request_panel_refresh(path.clone(), now));
    receiver.try_recv().unwrap();
    let timeout = now + FOLDER_SIZE_REQUEST_TIMEOUT;
    assert_eq!(state.expire_requests(timeout), vec![path.clone()]);
    assert_eq!(
        state.summary_for_panel_render(&path),
        (Some(confirmed), false)
    );
    assert!(!state.request_panel_refresh(path.clone(), timeout + Duration::from_secs(3)));
    let retry = timeout + FOLDER_SIZE_FAILURE_RETRY_DELAY;
    assert_eq!(
        state.take_due_panel_revalidation(retry, &path),
        Some(path.clone())
    );
    assert!(state.request_panel_refresh(path, retry));
    assert_eq!(receiver.try_iter().count(), 1);
}

#[test]
fn manual_refresh_can_bypass_debounce_without_removing_the_visible_summary() {
    let (mut state, receiver) = connected_state();
    let path = PathBuf::from("folder");
    let now = Instant::now();
    let confirmed = FolderContentSummary::complete(10, 1, 0);
    state.cache_completed_panel_summary(path.clone(), confirmed);
    state.reschedule_panel_revalidation_if_stale(&path, now);
    assert!(!state.request_panel_refresh(path.clone(), now));
    // Explicit refresh overrides only the automatic debounce deadline.
    state.panel_deferred_revalidation.insert(path.clone(), now);
    assert!(state.request_panel_refresh(path.clone(), now));
    assert_eq!(receiver.try_iter().count(), 1);
    assert_eq!(
        state.summary_for_panel_render(&path),
        (Some(confirmed), false)
    );
}
