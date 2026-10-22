use super::worker::read_dates;
use super::*;

fn result(path: &str, modified: u64) -> ReadResult {
    ReadResult {
        path: PathBuf::from(path),
        dates: Some(FolderDates {
            modified,
            created: Some(100),
        }),
    }
}

#[test]
fn known_dates_can_be_refreshed_and_repeated_events_are_coalesced() {
    let mut state = FolderTimestampState::default();
    let path = Path::new(r"C:\Folder");
    let now = Instant::now();
    for i in 0..4 {
        state.request(
            path,
            RefreshReason::Watcher,
            now + Duration::from_millis(i * 100),
        );
    }
    assert_eq!(state.pending.len(), 1);
    let due = now + DEBOUNCE + Duration::from_millis(300);
    assert!(state.take_due(due - Duration::from_millis(1)).is_empty());
    assert_eq!(state.take_due(due), [path.to_path_buf()]);
    assert_eq!(
        state.finish(vec![result(r"C:\Folder", 300)])[0].1.modified,
        300
    );
}

#[test]
fn watcher_waits_for_the_last_event_to_settle_before_reading() {
    let mut state = FolderTimestampState::default();
    let now = Instant::now();
    for i in 0..30 {
        state.request(
            Path::new(r"C:\Folder"),
            RefreshReason::Watcher,
            now + Duration::from_millis(i * 100),
        );
    }
    assert!(state.take_due(now + Duration::from_millis(2900)).is_empty());
    assert_eq!(
        state
            .take_due(now + Duration::from_millis(2900) + DEBOUNCE)
            .len(),
        1
    );
}

#[test]
fn f5_bypasses_debounce_and_later_watcher_events_do_not_delay_it() {
    let mut state = FolderTimestampState::default();
    let now = Instant::now();
    let path = Path::new(r"C:\Folder");
    state.request(path, RefreshReason::Watcher, now);
    let manual = now + Duration::from_millis(1);
    state.request(path, RefreshReason::Manual, manual);
    state.request(path, RefreshReason::Watcher, manual);
    assert_eq!(state.take_due(manual), [path.to_path_buf()]);
}

#[test]
fn events_and_f5_reject_in_flight_results_including_path_aliases() {
    for reason in [RefreshReason::Watcher, RefreshReason::Manual] {
        let mut state = FolderTimestampState::default();
        let now = Instant::now();
        let path = Path::new(r"C:\Folder");
        state.request(path, RefreshReason::Missing, now);
        assert_eq!(state.take_due(now).len(), 1);
        state.request(Path::new(r"\\?\c:\FOLDER"), reason, now);
        assert!(state.finish(vec![result(r"C:\Folder", 200)]).is_empty());
        assert_eq!(state.take_due(now + DEBOUNCE).len(), 1);
        assert_eq!(
            state.finish(vec![result(r"\\?\c:\FOLDER", 300)])[0]
                .1
                .modified,
            300
        );
    }
}

#[test]
fn navigation_deduplicates_existing_reads_without_invalidating_them() {
    let mut state = FolderTimestampState::default();
    let now = Instant::now();
    let path = Path::new(r"C:\Folder");
    state.request(path, RefreshReason::Missing, now);
    assert!(!state.request(path, RefreshReason::Missing, now));
    state.take_due(now);
    assert!(!state.request(path, RefreshReason::Missing, now));
    assert_eq!(state.finish(vec![result(r"C:\Folder", 300)]).len(), 1);
}

#[test]
fn pending_work_and_concurrent_reads_are_bounded() {
    let mut state = FolderTimestampState::default();
    let now = Instant::now();
    for i in 0..MAX_PENDING {
        assert!(state.request(
            &PathBuf::from(format!(r"C:\Folder\{i}")),
            RefreshReason::Manual,
            now
        ));
    }
    assert!(!state.request(Path::new(r"C:\Other"), RefreshReason::Manual, now));
    assert_eq!(state.pending.len(), MAX_PENDING);
    let paths = state.take_due(now);
    assert_eq!(paths.len(), BATCH_SIZE);
    assert!(state.take_due(now).is_empty());
    assert_eq!(state.in_flight.len(), BATCH_SIZE);
    let ready = state.finish(
        paths
            .into_iter()
            .map(|path| ReadResult { path, dates: None })
            .collect(),
    );
    assert!(ready.is_empty());
    assert_eq!(state.take_due(now).len(), BATCH_SIZE);
}

#[test]
fn an_unrelated_folders_event_does_not_discard_a_valid_result() {
    let mut state = FolderTimestampState::default();
    let now = Instant::now();
    state.request(Path::new(r"C:\Folder"), RefreshReason::Missing, now);
    state.take_due(now);
    state.request(Path::new(r"D:\Other"), RefreshReason::Watcher, now);
    assert_eq!(state.finish(vec![result(r"C:\Folder", 300)]).len(), 1);
}

fn read_batch(
    state: &mut FolderTimestampState,
    ctx: &egui::Context,
    now: Instant,
) -> Vec<(PathBuf, FolderDates)> {
    assert!(state.poll(ctx, now).is_empty());
    let results = state
        .worker
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    state.finish(results)
}

#[test]
fn worker_reads_actual_directory_dates_and_releases_failed_requests() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing");
    let mut state = FolderTimestampState::default();
    let now = Instant::now();
    state.request(dir.path(), RefreshReason::Missing, now);
    state.request(&missing, RefreshReason::Manual, now);
    let dates = read_batch(&mut state, &ctx, now);
    assert_eq!(
        dates,
        [(dir.path().to_path_buf(), read_dates(dir.path()).unwrap())]
    );
    assert!(state.in_flight.is_empty());
    assert!(state.request(&missing, RefreshReason::Manual, now));
}

#[cfg(windows)]
#[test]
fn watcher_and_f5_read_new_dates_even_after_the_first_successful_resolution() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_WRITE_ATTRIBUTES};
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES.0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(dir.path())
        .unwrap();
    let mut state = FolderTimestampState::default();
    let now = Instant::now();
    for (i, reason) in [
        RefreshReason::Missing,
        RefreshReason::Watcher,
        RefreshReason::Manual,
    ]
    .into_iter()
    .enumerate()
    {
        let expected = 1_600_000_000 + i as u64 * 100;
        file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(expected))
            .unwrap();
        state.request(dir.path(), reason, now);
        let dates = read_batch(&mut state, &ctx, now + DEBOUNCE);
        assert_eq!(dates[0].1.modified, expected);
    }
}
