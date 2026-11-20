use super::*;
use windows::Win32::UI::Shell::{
    COPYENGINE_S_ALREADY_DONE, COPYENGINE_S_CLOSE_PROGRAM, COPYENGINE_S_COLLISIONRESOLVED,
    COPYENGINE_S_DONT_PROCESS_CHILDREN, COPYENGINE_S_KEEP_BOTH, COPYENGINE_S_MERGE,
    COPYENGINE_S_NOT_HANDLED, COPYENGINE_S_PROGRESS_PAUSE, COPYENGINE_S_USER_IGNORED,
    COPYENGINE_S_YES,
};

#[test]
fn empty_recycle_bin_successful_nonzero_results_count_all_selected_roots_once() {
    for result in [
        S_OK,
        COPYENGINE_S_DONT_PROCESS_CHILDREN,
        COPYENGINE_S_ALREADY_DONE,
        COPYENGINE_S_COLLISIONRESOLVED,
    ] {
        let (job, tracking) = batch(3319);
        for index in 0..3319 {
            tracking.record(index, false, result);
            tracking.record(index, true, result);
            tracking.record(index, true, result);
        }
        let progress = job.operation.deletion.as_ref().unwrap();
        assert_eq!(progress.permanently_removed.load(Ordering::Relaxed), 3319);
        assert_eq!(progress.removed.load(Ordering::Relaxed), 3319);
        assert_eq!(progress.failures.load(Ordering::Relaxed), 0);
        assert_eq!(tracking.outcome(Ok(false)).kind, OutcomeKind::Completed);
    }
}

#[test]
fn empty_recycle_bin_informational_results_are_not_deletion_errors_or_success() {
    for result in [
        S_FALSE,
        COPYENGINE_S_PENDING,
        COPYENGINE_S_PENDING_DELETE,
        COPYENGINE_S_USER_RETRY,
        COPYENGINE_S_USER_IGNORED,
        COPYENGINE_S_NOT_HANDLED,
        COPYENGINE_S_PROGRESS_PAUSE,
        COPYENGINE_S_CLOSE_PROGRAM,
        COPYENGINE_S_YES,
        COPYENGINE_S_MERGE,
        COPYENGINE_S_KEEP_BOTH,
        windows::core::HRESULT(0x0027_0011), // Pending batch copy in newer SDKs.
        windows::core::HRESULT(0x0027_7FFF), // Unknown informational result.
    ] {
        let (job, tracking) = batch(1);
        tracking.record(0, true, result);
        let outcome = tracking.outcome(Ok(false));
        let progress = job.operation.deletion.as_ref().unwrap();
        assert_eq!(progress.permanently_removed.load(Ordering::Relaxed), 0);
        assert_eq!(progress.failures.load(Ordering::Relaxed), 0);
        assert_eq!(outcome.kind, OutcomeKind::Failed);
        assert_eq!(
            outcome.message.unwrap(),
            rust_i18n::t!("operations.empty_recycle_bin_unconfirmed").to_string()
        );
    }
}

#[test]
fn empty_recycle_bin_pending_then_completed_callbacks_count_the_final_result() {
    let (job, tracking) = batch(3);
    tracking.record(0, true, COPYENGINE_S_PENDING_DELETE);
    tracking.record(0, true, COPYENGINE_S_DONT_PROCESS_CHILDREN);
    tracking.record(1, false, E_ACCESSDENIED);
    tracking.record(1, true, COPYENGINE_S_DONT_PROCESS_CHILDREN);
    tracking.record(2, true, COPYENGINE_S_ALREADY_DONE);
    let outcome = tracking.outcome(Ok(false));
    let progress = job.operation.deletion.as_ref().unwrap();
    assert_eq!(progress.permanently_removed.load(Ordering::Relaxed), 2);
    assert_eq!(progress.failures.load(Ordering::Relaxed), 1);
    assert_eq!(outcome.kind, OutcomeKind::Failed);
    assert_eq!(
        outcome.message.unwrap(),
        rust_i18n::t!("delete_engine.failed", count = 1).to_string()
    );
}
