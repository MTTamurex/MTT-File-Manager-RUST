use super::*;

#[test]
fn stalled_output_does_not_block_capture_and_overflow_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("diagnostic.log");
    let writer = Writer::new();
    writer.open(1, path.clone(), SystemTime::now()).unwrap();
    let (started, ready) = mpsc::sync_channel(1);
    let (release, blocked) = mpsc::channel();
    writer
        .sender()
        .unwrap()
        .send(Command::Block(started, blocked))
        .unwrap();
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    let before = std::time::Instant::now();
    for _ in 0..NORMAL_PENDING_BATCHES + 10 {
        writer.events(1, vec![Event::new(Level::Info, "test", "capture", vec![])]);
    }
    assert_eq!(
        writer.counters.pending.load(Ordering::Acquire),
        NORMAL_PENDING_BATCHES
    );
    assert_eq!(writer.counters.dropped.load(Ordering::Acquire), 10);
    writer.critical_events(
        1,
        vec![Event::new(Level::Warn, "test", "slow_frame", vec![])],
    );
    writer.flush(Duration::from_millis(10));
    writer.close(1);
    assert!(before.elapsed() < Duration::from_secs(1));
    release.send(()).unwrap();
    writer.flush(Duration::from_secs(2));
    let contents = std::fs::read_to_string(path).unwrap();
    assert!(contents.contains("event=capture_overflow dropped_batches=10"));
    assert!(contents.contains("event=session_ended"));
    assert!(contents.contains("event=slow_frame"));
}

#[test]
fn asynchronous_open_failure_is_observable_once_per_session() {
    let dir = tempfile::tempdir().unwrap();
    let not_directory = dir.path().join("file.txt");
    std::fs::write(&not_directory, b"test").unwrap();
    let writer = Writer::new();
    writer
        .open(1, not_directory.join("diagnostic.log"), SystemTime::now())
        .unwrap();
    writer.flush(Duration::from_secs(2));
    assert!(writer.failures() > 0);
    assert!(writer.take_output_failure());
    assert!(!writer.take_output_failure());
}

#[test]
fn full_critical_queue_returns_the_original_report_for_later_retry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("diagnostic.log");
    let writer = Writer::new();
    writer.open(1, path.clone(), SystemTime::now()).unwrap();
    let (started, ready) = mpsc::sync_channel(1);
    let (release, blocked) = mpsc::channel();
    writer
        .sender()
        .unwrap()
        .send(Command::Block(started, blocked))
        .unwrap();
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    for _ in 0..MAX_PENDING_BATCHES {
        writer
            .try_critical_events(1, vec![Event::new(Level::Info, "test", "queued", vec![])])
            .unwrap();
    }
    let rejected = writer
        .try_critical_events(
            1,
            vec![Event::new(Level::Warn, "test", "preserved_stall", vec![])],
        )
        .unwrap_err();
    assert_eq!(rejected[0].code, "preserved_stall");
    assert_eq!(writer.counters.dropped.load(Ordering::Acquire), 0);
    assert_eq!(writer.counters.rejected.load(Ordering::Acquire), 1);
    release.send(()).unwrap();
    writer.flush(Duration::from_secs(2));
    writer.try_critical_events(1, rejected).unwrap();
    writer.close(1);
    writer.flush(Duration::from_secs(2));
    assert!(std::fs::read_to_string(path)
        .unwrap()
        .contains("event=preserved_stall"));
}
