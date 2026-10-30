use super::*;

#[test]
fn windows_stage_is_published_only_after_successful_checks() {
    let (_dir, request) = request();
    let progress = Mutex::new(DeleteProgress::default());
    let result =
        validation::execute_with_progress(&request, &AtomicBool::new(false), &progress, |_| {
            let snapshot = progress::snapshot(&progress);
            assert_eq!(snapshot.stage, DeleteStage::Windows);
            assert!(snapshot.failure.is_none());
            assert_eq!(snapshot.fraction(), None);
            false
        });
    assert_eq!(result, (DeleteStatus::CancelledOrFailed, true));
}

#[test]
fn failed_metadata_identifies_the_copy_and_leaves_later_steps_pending() {
    for survivor in [true, false] {
        let (dir, mut request) = request();
        if survivor {
            request.survivor_path = dir.path().join("missing.bin");
        } else {
            request.target_path = dir.path().join("missing.bin");
        }
        let progress = Mutex::new(DeleteProgress::default());
        let result =
            validation::execute_with_progress(&request, &AtomicBool::new(false), &progress, |_| {
                panic!("unsafe dispatch")
            });
        assert_eq!(result, (DeleteStatus::Unsafe, false));
        let snapshot = progress::snapshot(&progress);
        assert_eq!(snapshot.stage, DeleteStage::Metadata);
        assert_eq!(
            snapshot.failure,
            Some(if survivor {
                DeleteFailure::SurvivorUnavailableOrChanged
            } else {
                DeleteFailure::TargetUnavailableOrChanged
            })
        );
    }
}

#[test]
fn changed_contents_explain_the_failure_without_dispatching_windows() {
    use std::io::Write;
    let (_dir, request) = request();
    let modified = std::fs::metadata(&request.target_path)
        .unwrap()
        .modified()
        .unwrap();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(&request.target_path)
        .unwrap();
    file.write_all(&vec![9; 4096]).unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    drop(file);
    let progress = Mutex::new(DeleteProgress::default());
    let result =
        validation::execute_with_progress(&request, &AtomicBool::new(false), &progress, |_| {
            panic!("different content dispatched")
        });
    assert_eq!(result, (DeleteStatus::Unsafe, false));
    let snapshot = progress::snapshot(&progress);
    assert_eq!(snapshot.stage, DeleteStage::Revalidate);
    assert_eq!(snapshot.failure, Some(DeleteFailure::ContentMismatch));
}

#[test]
fn cancellation_is_not_reported_as_a_failed_safety_check() {
    let (_dir, request) = request();
    let progress = Mutex::new(DeleteProgress::default());
    let result =
        validation::execute_with_progress(&request, &AtomicBool::new(true), &progress, |_| {
            panic!("cancelled dispatch")
        });
    assert_eq!(result, (DeleteStatus::CancelledOrFailed, false));
    assert!(progress::snapshot(&progress).failure.is_none());
    assert_eq!(progress::snapshot(&progress).stage, DeleteStage::Pair);
}
