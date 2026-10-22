use super::*;

#[test]
fn manual_refresh_is_dispatched_before_an_older_watcher_backlog() {
    let mut state = FolderTimestampState::default();
    let now = Instant::now();
    for i in 0..BATCH_SIZE * 2 {
        state.request(
            &PathBuf::from(format!(r"C:\Folder\{i}")),
            RefreshReason::Watcher,
            now,
        );
    }
    let current = Path::new(r"D:\Current");
    let manual = now + DEBOUNCE + Duration::from_secs(1);
    state.request(current, RefreshReason::Manual, manual);
    let paths = state.take_due(manual);
    assert_eq!(paths.first(), Some(&current.to_path_buf()));
    assert_eq!(paths.len(), BATCH_SIZE);
}

#[test]
fn a_full_watcher_queue_does_not_drop_the_users_manual_refresh() {
    let mut state = FolderTimestampState::default();
    let now = Instant::now();
    for i in 0..MAX_PENDING {
        assert!(state.request(
            &PathBuf::from(format!(r"C:\Folder\{i}")),
            RefreshReason::Watcher,
            now
        ));
    }
    let current = Path::new(r"D:\Current");
    assert!(state.request(current, RefreshReason::Manual, now));
    assert_eq!(state.pending.len(), MAX_PENDING);
    assert_eq!(state.take_due(now), [current.to_path_buf()]);
}

#[test]
fn disconnected_worker_releases_the_batch_and_keeps_pending_work() {
    let mut state = FolderTimestampState::default();
    let now = Instant::now();
    let path = Path::new(r"C:\Folder");
    state.request(path, RefreshReason::Missing, now);
    state.take_due(now);
    let (sender, receiver) = mpsc::channel();
    state.worker = Some(receiver);
    drop(sender);
    state.request(Path::new(r"D:\Other"), RefreshReason::Watcher, now);
    assert!(state.poll(&egui::Context::default(), now).is_empty());
    assert!(state.worker.is_none());
    assert!(state.in_flight.is_empty());
    assert_eq!(state.next_wakeup(now), Some(DEBOUNCE));
    assert!(state.request(path, RefreshReason::Manual, now));
}

#[test]
fn polling_a_completed_batch_returns_dates_without_an_idle_repaint_loop() {
    let mut state = FolderTimestampState::default();
    let now = Instant::now();
    let path = PathBuf::from(r"C:\Folder");
    let dates = FolderDates {
        modified: 300,
        created: Some(100),
    };
    state.request(&path, RefreshReason::Missing, now);
    state.take_due(now);
    let (sender, receiver) = mpsc::channel();
    state.worker = Some(receiver);
    sender
        .send(vec![ReadResult {
            path: path.clone(),
            dates: Some(dates),
        }])
        .unwrap();
    drop(sender);
    assert_eq!(state.poll(&egui::Context::default(), now), [(path, dates)]);
    assert!(state.worker.is_none());
    assert!(state.in_flight.is_empty());
    assert_eq!(state.next_wakeup(now), None);
}
