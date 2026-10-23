use super::*;

#[test]
fn virtual_lane_reads_folder_dates_without_prior_selection_or_cached_hints() {
    let mut queues = FolderTimestampQueues::default();
    let ctx = egui::Context::default();
    let directory = tempfile::tempdir().unwrap();
    let now = Instant::now();
    assert!(queues.request(
        directory.path(),
        RefreshReason::Missing,
        now,
        ReadLane::Virtual
    ));
    assert!(queues.poll(&ctx, now).is_empty());
    let results = queues
        .virtual_drives
        .worker
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    let dates = queues.virtual_drives.finish(results);
    assert_eq!(
        dates,
        [(
            directory.path().to_path_buf(),
            worker::read_dates(directory.path()).unwrap()
        )]
    );
    assert!(queues.local.worker.is_none());
    assert!(queues.local.in_flight.is_empty());
}

#[test]
fn virtual_missing_dates_are_queued_and_duplicate_requests_are_coalesced() {
    let mut queues = FolderTimestampQueues::default();
    let path = Path::new(r"Z:\Folder");
    let now = Instant::now();
    assert!(queues.request(path, RefreshReason::Missing, now, ReadLane::Virtual));
    assert!(!queues.request(path, RefreshReason::Missing, now, ReadLane::Virtual));
    assert!(queues.local.pending.is_empty());
    assert_eq!(queues.virtual_drives.take_due(now), [path.to_path_buf()]);
    assert!(!queues.request(path, RefreshReason::Missing, now, ReadLane::Virtual));
    let dates = FolderDates {
        modified: 200,
        created: Some(100),
    };
    assert_eq!(
        queues.virtual_drives.finish(vec![ReadResult {
            path: path.to_path_buf(),
            dates: Some(dates),
        }]),
        [(path.to_path_buf(), dates)]
    );
}

#[test]
fn stalled_virtual_read_does_not_prevent_local_results_or_new_local_reads() {
    let mut queues = FolderTimestampQueues::default();
    let now = Instant::now();
    let virtual_path = Path::new(r"Z:\Folder");
    queues.request(virtual_path, RefreshReason::Missing, now, ReadLane::Virtual);
    queues.virtual_drives.take_due(now);
    // Keep the sender alive without replying to simulate a blocked driver.
    let (_blocked_sender, blocked_receiver) = mpsc::channel();
    queues.virtual_drives.worker = Some(blocked_receiver);

    let local_path = Path::new(r"C:\Folder");
    queues.request(local_path, RefreshReason::Missing, now, ReadLane::Local);
    assert_eq!(queues.local.take_due(now), [local_path.to_path_buf()]);
    let (sender, receiver) = mpsc::channel();
    queues.local.worker = Some(receiver);
    let dates = FolderDates {
        modified: 300,
        created: Some(150),
    };
    sender
        .send(vec![ReadResult {
            path: local_path.to_path_buf(),
            dates: Some(dates),
        }])
        .unwrap();
    assert_eq!(
        queues.poll(&egui::Context::default(), now),
        [(local_path.to_path_buf(), dates)]
    );
    assert!(queues.virtual_drives.worker.is_some());
    assert_eq!(queues.virtual_drives.in_flight.len(), 1);
    let next = Path::new(r"D:\Folder");
    assert!(queues.request(next, RefreshReason::Manual, now, ReadLane::Local));
    assert_eq!(queues.local.take_due(now), [next.to_path_buf()]);
}

#[test]
fn virtual_f5_keeps_manual_priority_over_watcher_debounce() {
    let mut queues = FolderTimestampQueues::default();
    let now = Instant::now();
    let path = Path::new(r"Z:\Folder");
    queues.request(path, RefreshReason::Watcher, now, ReadLane::Virtual);
    assert!(queues.virtual_drives.take_due(now).is_empty());
    queues.request(path, RefreshReason::Manual, now, ReadLane::Virtual);
    assert_eq!(queues.virtual_drives.take_due(now), [path.to_path_buf()]);
    assert!(queues.local.pending.is_empty());
}
