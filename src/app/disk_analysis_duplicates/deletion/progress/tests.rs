use super::*;

#[test]
fn progress_is_throttled_but_completion_and_stage_changes_are_immediate() {
    let shared = Mutex::new(DeleteProgress::default());
    let mut reporter = Reporter::new(&shared);
    reporter.begin(DeleteStage::TargetContent, 100);
    // Use a future timestamp to make the throttle test independent of scheduling.
    reporter.last_update = Instant::now() + Duration::from_secs(60);
    reporter.read(10);
    assert_eq!(snapshot(&shared).bytes_read, 0);
    reporter.last_update = Instant::now() - UPDATE_INTERVAL;
    reporter.read(50);
    assert_eq!(snapshot(&shared).bytes_read, 50);
    reporter.read(100);
    assert_eq!(snapshot(&shared).bytes_read, 100);
    assert_eq!(snapshot(&shared).fraction(), Some(1.0));
    reporter.begin(DeleteStage::SurvivorContent, 100);
    assert_eq!(snapshot(&shared).stage, DeleteStage::SurvivorContent);
    assert_eq!(snapshot(&shared).bytes_read, 0);
    reporter.fail(DeleteFailure::SurvivorReadFailed);
    assert_eq!(
        snapshot(&shared).failure,
        Some(DeleteFailure::SurvivorReadFailed)
    );
    reporter.begin(DeleteStage::Windows, 0);
    assert!(snapshot(&shared).failure.is_none());
    assert_eq!(snapshot(&shared).fraction(), None);
}

#[test]
fn percentages_handle_empty_and_very_large_files_without_overflow() {
    let mut progress = DeleteProgress {
        stage: DeleteStage::TargetContent,
        ..Default::default()
    };
    assert_eq!(progress.fraction(), Some(1.0));
    progress.total_bytes = u64::MAX;
    progress.bytes_read = u64::MAX / 2;
    assert!((progress.fraction().unwrap() - 0.5).abs() < 0.001);
    progress.total_bytes = 1;
    assert_eq!(progress.fraction(), Some(1.0));
}
