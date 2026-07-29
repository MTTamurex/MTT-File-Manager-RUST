use super::*;
use std::num::NonZeroUsize;

fn test_state() -> FolderSizeState {
    let (req_sender, _req_receiver) = std::sync::mpsc::channel();
    let (_res_sender, res_receiver) = std::sync::mpsc::channel();
    let (batch_req_sender, _batch_req_receiver) = std::sync::mpsc::channel();
    let (_batch_res_sender, batch_res_receiver) = std::sync::mpsc::channel();

    FolderSizeState {
        req_sender,
        res_receiver,
        cancel: Arc::new(AtomicBool::new(false)),
        latest_request_id: Arc::new(AtomicU64::new(0)),
        cache: LruCache::new(NonZeroUsize::new(8).unwrap()),
        loading: FxHashSet::default(),
        active_requests: HashMap::new(),
        next_request_id: 0,
        failed_until: HashMap::new(),
        panel_stale_cache: LruCache::new(NonZeroUsize::new(8).unwrap()),
        panel_deferred_revalidation: HashMap::new(),
        batch_req_sender,
        batch_res_receiver,
        batch_cancel: Arc::new(AtomicBool::new(false)),
        batch_generation: Arc::new(AtomicU64::new(0)),
        batch_loading: FxHashSet::default(),
        batch_cache: LruCache::new(NonZeroUsize::new(8).unwrap()),
        pending_revalidation: HashMap::new(),
        pending_revalidation_next_deadline: None,
        batch_invalidation_epoch: HashMap::new(),
        batch_invalidation_last_prune: Instant::now(),
    }
}

#[test]
fn panel_stale_summary_requires_complete_counts() {
    let mut state = test_state();
    let path = PathBuf::from(r"C:\data");
    let now = Instant::now();

    state.preserve_panel_summary_for_deferred_revalidation(
        path.clone(),
        FolderContentSummary::size_only(10),
        now,
    );
    assert!(state.panel_stale_cache.peek(&path).is_none());

    let summary = FolderContentSummary::complete(10, 2, 1);
    state.preserve_panel_summary_for_deferred_revalidation(path.clone(), summary, now);
    assert_eq!(state.panel_stale_cache.peek(&path).copied(), Some(summary));
}

#[test]
fn panel_render_uses_stale_summary_without_loading_state() {
    let mut state = test_state();
    let path = PathBuf::from(r"C:\data");
    let stale = FolderContentSummary::complete(100, 12, 3);
    state.preserve_panel_summary_for_deferred_revalidation(path.clone(), stale, Instant::now());
    state
        .cache
        .put(path.clone(), FolderContentSummary::size_only(50));
    state.loading.insert(path.clone());

    let (summary, loading) = state.summary_for_panel_render(&path, true);

    assert_eq!(summary, Some(stale));
    assert!(!loading);
}

#[test]
fn folder_size_failure_expires_after_retry_delay() {
    let mut state = test_state();
    let path = PathBuf::from(r"C:\data");
    let now = Instant::now();

    state.record_failure(path.clone(), now);

    assert!(state.is_failure_active(&path, now + Duration::from_secs(1)));
    assert!(!state.is_failure_active(
        &path,
        now + FOLDER_SIZE_FAILURE_RETRY_DELAY + Duration::from_millis(1)
    ));
}

#[test]
fn panel_deferred_revalidation_waits_for_deadline() {
    let mut state = test_state();
    let path = PathBuf::from(r"C:\data");
    let other = PathBuf::from(r"C:\other");
    let now = Instant::now();

    state.preserve_panel_summary_for_deferred_revalidation(
        path.clone(),
        FolderContentSummary::complete(100, 12, 3),
        now,
    );

    assert_eq!(
        state.take_due_panel_revalidation(
            now + PANEL_STALE_REVALIDATION_DELAY - Duration::from_millis(1),
            &path,
        ),
        None
    );
    assert_eq!(
        state.take_due_panel_revalidation(
            now + PANEL_STALE_REVALIDATION_DELAY + Duration::from_millis(1),
            &other,
        ),
        None
    );
    assert_eq!(
        state.take_due_panel_revalidation(
            now + PANEL_STALE_REVALIDATION_DELAY + Duration::from_millis(1),
            &path,
        ),
        Some(path.clone())
    );
    assert_eq!(
        state.take_due_panel_revalidation(
            now + PANEL_STALE_REVALIDATION_DELAY + Duration::from_millis(2),
            &path,
        ),
        None
    );
}

#[test]
fn folder_size_revalidation_expires_once() {
    let mut state = test_state();
    let path = PathBuf::from(r"C:\data");
    let start = Instant::now();
    let delay = state.schedule_revalidation(path.clone(), Some(17), start);

    assert_eq!(delay, Duration::from_secs(3));
    assert!(state
        .take_expired_revalidations(start + Duration::from_secs(2))
        .is_empty());

    let deadline = start + Duration::from_secs(3);
    assert_eq!(
        state.take_expired_revalidations(deadline),
        vec![(path.clone(), false)]
    );
    assert!(!state.pending_revalidation.contains_key(&path));
    assert!(state
        .take_expired_revalidations(deadline + Duration::from_secs(30))
        .is_empty());
}

#[test]
fn fresh_changed_result_cancels_deferred_revalidation() {
    let mut state = test_state();
    let path = PathBuf::from(r"C:\data");
    state.schedule_revalidation(path.clone(), Some(17), Instant::now());

    state.cancel_revalidation_if_changed(&path, 0);

    assert!(!state.pending_revalidation.contains_key(&path));
    assert!(state.pending_revalidation_next_deadline.is_none());
}

#[test]
fn batch_cancel_leaves_safe_lazy_deadline() {
    let mut state = test_state();
    let first = PathBuf::from(r"C:\first");
    let second = PathBuf::from(r"C:\second");
    let start = Instant::now();
    state.schedule_revalidation(first.clone(), Some(10), start);
    state.schedule_revalidation(second.clone(), Some(20), start + Duration::from_secs(1));

    state.cancel_revalidations(std::slice::from_ref(&first));

    assert!(!state.pending_revalidation.contains_key(&first));
    assert!(state.pending_revalidation.contains_key(&second));
    assert_eq!(
        state.pending_revalidation_next_deadline,
        Some(start + Duration::from_secs(3))
    );
    assert!(state.should_prune_pending_revalidations(start + Duration::from_secs(3)));
    assert!(state
        .take_expired_revalidations(start + Duration::from_secs(3))
        .is_empty());
    assert_eq!(
        state.pending_revalidation_next_deadline,
        Some(start + Duration::from_secs(4))
    );
}

#[test]
fn failed_batch_revalidation_releases_loading_for_retry() {
    let mut state = test_state();
    let path = PathBuf::from(r"C:\failed");
    let start = Instant::now();
    let delay = state.schedule_revalidation_if_absent(path.clone(), start);

    assert_eq!(
        state.take_expired_revalidations(start + delay),
        vec![(path, true)]
    );
}

#[test]
fn failed_request_send_does_not_leave_loading_or_active_state() {
    let mut state = test_state();
    let path = PathBuf::from(r"C:\disconnected");
    let now = Instant::now();

    assert!(!state.dispatch_request(path.clone(), 0, now));
    assert!(!state.loading.contains(&path));
    assert!(!state.active_requests.contains_key(&path));
    assert!(state.is_failure_active(&path, now));
}

#[test]
fn expired_request_releases_loading_and_records_failure() {
    let mut state = test_state();
    let (sender, receiver) = std::sync::mpsc::channel();
    state.req_sender = sender;
    let path = PathBuf::from(r"C:\slow");
    let now = Instant::now();

    assert!(state.dispatch_request(path.clone(), 3, now));
    let request = receiver.recv().unwrap();
    assert_eq!(request.folder_path, path);
    assert!(state.is_active_request(&path, request.request_id));
    assert!(state.loading.contains(&path));

    assert!(state
        .expire_requests(now + FOLDER_SIZE_REQUEST_TIMEOUT - Duration::from_millis(1))
        .is_empty());
    assert_eq!(
        state.expire_requests(now + FOLDER_SIZE_REQUEST_TIMEOUT),
        vec![path.clone()]
    );
    assert!(!state.loading.contains(&path));
    assert!(!state.active_requests.contains_key(&path));
    assert!(state.is_failure_active(&path, now + FOLDER_SIZE_REQUEST_TIMEOUT));
}

#[test]
fn old_request_id_cannot_finish_newer_request_for_same_path() {
    let mut state = test_state();
    let (sender, receiver) = std::sync::mpsc::channel();
    state.req_sender = sender;
    let path = PathBuf::from(r"C:\changing");
    let now = Instant::now();

    assert!(state.dispatch_request(path.clone(), 1, now));
    let old = receiver.recv().unwrap();
    assert!(state.dispatch_request(path.clone(), 2, now));
    let new = receiver.recv().unwrap();

    assert!(!state.finish_request(&path, old.request_id));
    assert_eq!(
        state.latest_request_id.load(Ordering::Acquire),
        new.request_id
    );
    assert!(state.is_active_request(&path, new.request_id));
    assert!(state.finish_request(&path, new.request_id));
    assert_eq!(state.latest_request_id.load(Ordering::Acquire), 0);
    assert!(!state.loading.contains(&path));
}

#[test]
fn cancelling_active_request_releases_loading_and_signals_worker() {
    let mut state = test_state();
    let (sender, receiver) = std::sync::mpsc::channel();
    state.req_sender = sender;
    let path = PathBuf::from(r"C:\invalidated");

    assert!(state.dispatch_request(path.clone(), 1, Instant::now()));
    let _ = receiver.recv().unwrap();
    assert!(state.cancel_active_request(&path));
    assert!(!state.loading.contains(&path));
    assert!(!state.active_requests.contains_key(&path));
    assert!(state.cancel.load(Ordering::Acquire));
    assert_eq!(state.latest_request_id.load(Ordering::Acquire), 0);
}
