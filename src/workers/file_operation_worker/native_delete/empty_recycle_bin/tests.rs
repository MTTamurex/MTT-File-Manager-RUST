use super::*;
use crate::infrastructure::delete_engine::confirmation::tests::wait_for_request;
use std::sync::atomic::Ordering;
use windows::Win32::Foundation::{E_ABORT, E_ACCESSDENIED, S_FALSE, S_OK};
use windows::Win32::UI::Shell::{
    COPYENGINE_E_USER_CANCELLED, COPYENGINE_S_DONT_PROCESS_CHILDREN, COPYENGINE_S_PENDING,
    COPYENGINE_S_PENDING_DELETE, COPYENGINE_S_USER_RETRY,
};
mod status_results;

fn batch(
    total: usize,
) -> (
    crate::infrastructure::copy_engine::CopyJob,
    tracking::Tracking,
) {
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete(Vec::new(), true);
    job.operation
        .deletion
        .as_ref()
        .unwrap()
        .total
        .store(total as u64, Ordering::Relaxed);
    let tracking = tracking::Tracking::new(job.operation.clone(), total);
    (job, tracking)
}

#[test]
fn themed_empty_bin_confirmation_carries_item_count_without_paths() {
    let (job, _) = batch(7);
    let operation = job.operation.clone();
    let worker = std::thread::spawn(move || confirmation::ask_empty_recycle_bin(&operation, 7));
    let request = wait_for_request(&job.operation);
    assert_eq!(request.kind, confirmation::Kind::EmptyRecycleBin);
    assert_eq!(request.total, 7);
    assert!(request.paths.is_empty());
    assert!(job
        .operation
        .deletion
        .as_ref()
        .unwrap()
        .confirmation
        .respond(request.id, false));
    assert!(!worker.join().unwrap());
}

#[test]
fn empty_recycle_bin_recursive_and_duplicate_callbacks_do_not_inflate_root_counts() {
    let (job, tracking) = batch(1);
    for _ in 0..10 {
        tracking.record(0, false, S_OK);
    }
    let progress = job.operation.deletion.as_ref().unwrap();
    assert_eq!(progress.permanently_removed.load(Ordering::Relaxed), 0);
    tracking.record(0, true, S_OK);
    tracking.record(0, true, S_OK);
    assert_eq!(progress.permanently_removed.load(Ordering::Relaxed), 1);
    assert_eq!(tracking.outcome(Ok(false)).kind, OutcomeKind::Completed);
}

#[test]
fn empty_recycle_bin_child_failures_count_the_selected_folder_once() {
    let (job, tracking) = batch(1);
    tracking.record(0, false, E_ACCESSDENIED);
    tracking.record(0, false, E_ACCESSDENIED);
    tracking.record(0, true, E_ACCESSDENIED);
    assert_eq!(tracking.outcome(Ok(true)).kind, OutcomeKind::Failed);
    let progress = job.operation.deletion.as_ref().unwrap();
    assert_eq!(progress.failures.load(Ordering::Relaxed), 1);
    assert_eq!(progress.permanently_removed.load(Ordering::Relaxed), 0);
}

#[test]
fn empty_recycle_bin_unreported_items_are_unconfirmed_not_deletion_errors() {
    let (job, tracking) = batch(3);
    tracking.record(0, true, S_OK);
    let outcome = tracking.outcome(Ok(false));
    assert_eq!(outcome.kind, OutcomeKind::Failed);
    assert_eq!(
        outcome.message.unwrap(),
        rust_i18n::t!("operations.empty_recycle_bin_unconfirmed").to_string()
    );
    assert_eq!(
        job.operation
            .deletion
            .as_ref()
            .unwrap()
            .failures
            .load(Ordering::Relaxed),
        0
    );
    // Terminal classification is idempotent.
    tracking.outcome(Ok(false));
    assert_eq!(
        job.operation
            .deletion
            .as_ref()
            .unwrap()
            .failures
            .load(Ordering::Relaxed),
        0
    );
}

#[test]
fn empty_recycle_bin_cancelled_items_remain_pending_without_fake_failures() {
    let (job, tracking) = batch(3);
    tracking.record(0, true, S_OK);
    tracking.record(1, true, COPYENGINE_E_USER_CANCELLED);
    assert_eq!(tracking.outcome(Ok(true)).kind, OutcomeKind::Cancelled);
    let progress = job.operation.deletion.as_ref().unwrap();
    assert_eq!(progress.failures.load(Ordering::Relaxed), 0);
    assert_eq!(progress.permanently_removed.load(Ordering::Relaxed), 1);
}

#[test]
fn empty_recycle_bin_global_finish_error_is_not_masked_by_aborted_bit() {
    let (job, tracking) = batch(1);
    tracking.finish_result(E_ACCESSDENIED);
    assert_eq!(tracking.outcome(Ok(true)).kind, OutcomeKind::Failed);
    assert_eq!(
        job.operation
            .deletion
            .as_ref()
            .unwrap()
            .failures
            .load(Ordering::Relaxed),
        0
    );
}

#[test]
fn empty_recycle_bin_nonterminal_and_skip_statuses_never_count_as_removals() {
    for status in [
        S_FALSE,
        COPYENGINE_S_PENDING,
        COPYENGINE_S_PENDING_DELETE,
        COPYENGINE_S_USER_RETRY,
    ] {
        let (job, tracking) = batch(1);
        tracking.record(0, true, status);
        assert_eq!(tracking.outcome(Ok(false)).kind, OutcomeKind::Failed);
        assert_eq!(
            job.operation
                .deletion
                .as_ref()
                .unwrap()
                .permanently_removed
                .load(Ordering::Relaxed),
            0
        );
    }
}

#[test]
fn empty_recycle_bin_error_without_callbacks_is_a_failure() {
    let (_, tracking) = batch(1);
    assert_eq!(
        tracking
            .outcome(Err(windows::core::Error::from_hresult(E_ACCESSDENIED)))
            .kind,
        OutcomeKind::Failed,
    );
}

#[test]
fn empty_recycle_bin_cancelled_global_sink_rejects_progress_callbacks() {
    let (job, tracking) = batch(1);
    let sink = sink::global(Arc::new(tracking));
    job.operation.progress.cancel();
    assert!(unsafe { sink.UpdateProgress(100, 10) }.is_err());
    assert!(unsafe { sink.PreDeleteItem(0, None) }.is_err());
}

#[test]
fn empty_recycle_bin_global_sink_does_not_count_item_notifications() {
    let (job, tracking) = batch(1);
    let tracking = Arc::new(tracking);
    let sink = sink::global(tracking.clone());
    unsafe { sink.PostDeleteItem(0, None, S_OK, None) }.unwrap();
    assert_eq!(
        job.operation
            .deletion
            .as_ref()
            .unwrap()
            .permanently_removed
            .load(Ordering::Relaxed),
        0
    );
    unsafe { sink.FinishOperations(E_ACCESSDENIED) }.unwrap();
    assert_eq!(tracking.outcome(Ok(true)).kind, OutcomeKind::Failed);
}

#[test]
fn empty_recycle_bin_cancellation_does_not_hide_a_racing_finish_error() {
    let (job, tracking) = batch(1);
    tracking.finish_result(E_ACCESSDENIED);
    job.operation.progress.cancel();
    assert_eq!(
        tracking
            .outcome(Err(windows::core::Error::from_hresult(E_ABORT)))
            .kind,
        OutcomeKind::Failed,
    );
}

#[test]
fn empty_recycle_bin_cancelled_enumeration_does_not_enter_the_shell() {
    let result = recycle_bin::enumerate_recycle_bin_shell_items(|| true);
    assert!(result.is_err_and(|error| error.code() == E_ABORT));
}

#[test]
fn empty_recycle_bin_concurrent_requests_are_ignored_and_guard_releases_on_unwind() {
    let runtime = CopyRuntime::default();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = Running::acquire(&runtime).unwrap();
        assert!(Running::acquire(&runtime).is_none());
        panic!("simulated worker failure");
    }));
    assert!(Running::acquire(&runtime).is_some());
}

#[test]
fn empty_recycle_bin_refresh_is_sent_even_on_unwind() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let _ = std::panic::catch_unwind(|| {
        let _refresh = RefreshOnDrop(&sender);
        panic!("simulated worker failure");
    });
    assert!(matches!(
        receiver.recv().unwrap(),
        FileOperationResult::RecycleBinChanged
    ));
    assert!(receiver.try_recv().is_err());
}

#[test]
fn empty_recycle_bin_item_sink_distinguishes_a_folder_from_its_descendants() {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{IShellItem, SHCreateItemFromParsingName};

    let _com = recycle_bin::ComApartmentGuard::init_sta_best_effort();
    let directory = tempfile::tempdir().unwrap();
    let child = directory.path().join("file.txt");
    std::fs::write(&child, b"test").unwrap();
    let shell_item = |path: &std::path::Path| -> IShellItem {
        let name: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe { SHCreateItemFromParsingName(PCWSTR(name.as_ptr()), None) }.unwrap()
    };
    let root = shell_item(directory.path());
    let root_alias = shell_item(directory.path());
    let child_item = shell_item(&child);
    let (job, tracking) = batch(1);
    let tracking = Arc::new(tracking);
    let sink = sink::item(tracking.clone(), 0, &root);
    // Simulate notifications only. No Shell deletion API is invoked.
    unsafe {
        sink.PreDeleteItem(0, &root).unwrap();
        sink.PreDeleteItem(0, &child_item).unwrap();
        sink.PostDeleteItem(0, &child_item, S_OK, None).unwrap();
    }
    assert_eq!(
        job.operation
            .deletion
            .as_ref()
            .unwrap()
            .permanently_removed
            .load(Ordering::Relaxed),
        0
    );
    std::fs::remove_file(&child).unwrap();
    std::fs::remove_dir(directory.path()).unwrap();
    unsafe {
        // Compare cannot be relied on here: the root has already disappeared.
        sink.PostDeleteItem(0, &root_alias, COPYENGINE_S_DONT_PROCESS_CHILDREN, None)
            .unwrap();
        sink.PostDeleteItem(0, &root, COPYENGINE_S_DONT_PROCESS_CHILDREN, None)
            .unwrap();
    }
    assert_eq!(tracking.outcome(Ok(false)).kind, OutcomeKind::Completed);
    assert_eq!(
        job.operation
            .deletion
            .as_ref()
            .unwrap()
            .permanently_removed
            .load(Ordering::Relaxed),
        1
    );
    assert!(!child.exists());
}

#[test]
fn empty_recycle_bin_item_sink_does_not_assume_callback_order_or_object_instance() {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{IShellItem, SHCreateItemFromParsingName};

    let _com = recycle_bin::ComApartmentGuard::init_sta_best_effort();
    let directory = tempfile::tempdir().unwrap();
    let child = directory.path().join("file.txt");
    std::fs::write(&child, b"test").unwrap();
    let shell_item = |path: &std::path::Path| -> IShellItem {
        let name: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe { SHCreateItemFromParsingName(PCWSTR(name.as_ptr()), None) }.unwrap()
    };
    let root = shell_item(directory.path());
    let root_alias = shell_item(directory.path());
    let child_item = shell_item(&child);
    for child_first in [false, true] {
        let (job, tracking) = batch(1);
        let tracking = Arc::new(tracking);
        let sink = sink::item(tracking.clone(), 0, &root);
        let progress = job.operation.deletion.as_ref().unwrap();
        unsafe {
            if child_first {
                // The first callback is not necessarily the selected root.
                sink.PreDeleteItem(0, &child_item).unwrap();
                sink.PostDeleteItem(0, &child_item, S_OK, None).unwrap();
                assert_eq!(progress.permanently_removed.load(Ordering::Relaxed), 0);
                sink.PreDeleteItem(0, &root).unwrap();
            } else {
                sink.PreDeleteItem(0, &root).unwrap();
                sink.PreDeleteItem(0, &child_item).unwrap();
            }
            // Match a separate Shell object for the root, not the last Pre.
            sink.PostDeleteItem(0, &root_alias, S_OK, None).unwrap();
            assert_eq!(progress.permanently_removed.load(Ordering::Relaxed), 1);
            if !child_first {
                sink.PostDeleteItem(0, &child_item, S_OK, None).unwrap();
            }
            sink.PostDeleteItem(0, &root_alias, S_OK, None).unwrap();
        }
        assert_eq!(tracking.outcome(Ok(false)).kind, OutcomeKind::Completed);
        assert_eq!(progress.permanently_removed.load(Ordering::Relaxed), 1);
        assert_eq!(progress.failures.load(Ordering::Relaxed), 0);
    }
}
