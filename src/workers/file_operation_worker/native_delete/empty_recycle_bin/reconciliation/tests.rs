use super::*;
use crate::infrastructure::copy_engine::{CopyJob, CopyRuntime, OutcomeKind};
use std::sync::atomic::Ordering;
use windows::core::Error;
use windows::Win32::Foundation::{E_ABORT, E_ACCESSDENIED, S_OK};

fn batch(total: usize) -> (CopyJob, Tracking, Snapshot) {
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete(Vec::new(), true);
    job.operation
        .deletion
        .as_ref()
        .unwrap()
        .total
        .store(total as u64, Ordering::Relaxed);
    let tracking = Tracking::new(job.operation.clone(), total);
    let snapshot = Snapshot {
        names: (0..total)
            .map(|index| Some(format!("entry-{index}")))
            .collect(),
    };
    (job, tracking, snapshot)
}

fn assert_counts(job: &CopyJob, removed: u64, failures: u64) {
    let progress = job.operation.deletion.as_ref().unwrap();
    assert_eq!(
        progress.permanently_removed.load(Ordering::Relaxed),
        removed
    );
    assert_eq!(progress.removed.load(Ordering::Relaxed), removed);
    assert_eq!(progress.failures.load(Ordering::Relaxed), failures);
}

#[test]
fn empty_recycle_bin_final_empty_snapshot_completes_five_of_eight_callbacks() {
    let (job, tracking, snapshot) = batch(8);
    for index in 0..5 {
        tracking.record(index, true, S_OK);
    }
    assert_counts(&job, 5, 0);
    for _ in 0..2 {
        snapshot.reconcile_with(&tracking, || Ok(Snapshot { names: Vec::new() }));
        assert_counts(&job, 8, 0);
        let outcome = tracking.outcome(Ok(false));
        assert_eq!(outcome.kind, OutcomeKind::Completed);
        assert!(outcome.message.is_none());
    }
}

#[test]
fn empty_recycle_bin_final_snapshot_only_counts_absent_selected_entries() {
    let (job, tracking, snapshot) = batch(3);
    // One selected root remains and another process has added an unrelated root.
    snapshot.reconcile_with(&tracking, || {
        Ok(Snapshot {
            names: vec![Some("entry-1".to_string()), Some("new-entry".to_string())],
        })
    });
    assert_counts(&job, 2, 0);
    assert_eq!(tracking.outcome(Ok(false)).kind, OutcomeKind::Failed);
}

#[test]
fn empty_recycle_bin_final_verification_failure_does_not_invent_removals() {
    let (job, tracking, snapshot) = batch(8);
    for index in 0..5 {
        tracking.record(index, true, S_OK);
    }
    snapshot.reconcile_with(&tracking, || Err(Error::from_hresult(E_ACCESSDENIED)));
    assert_counts(&job, 5, 0);
    assert_eq!(tracking.outcome(Ok(false)).kind, OutcomeKind::Failed);
}

#[test]
fn empty_recycle_bin_final_unknown_identities_do_not_prove_absence() {
    let (job, tracking, mut snapshot) = batch(3);
    snapshot.names[0] = None;
    snapshot.reconcile_with(&tracking, || {
        Ok(Snapshot {
            names: vec![None, Some("entry-1".to_string())],
        })
    });
    assert_counts(&job, 0, 0);
    snapshot.reconcile_with(&tracking, || {
        Ok(Snapshot {
            names: vec![Some("entry-1".to_string())],
        })
    });
    // The unknown original identity stays unconfirmed while any roots remain.
    assert_counts(&job, 1, 0);
    snapshot.reconcile_with(&tracking, || Ok(Snapshot { names: Vec::new() }));
    assert_counts(&job, 3, 0);
    assert_eq!(tracking.outcome(Ok(false)).kind, OutcomeKind::Completed);
}

#[test]
fn empty_recycle_bin_final_presence_overrules_a_success_callback_without_underflow() {
    let (job, tracking, snapshot) = batch(1);
    tracking.record(0, true, S_OK);
    for _ in 0..2 {
        snapshot.reconcile_with(&tracking, || {
            Ok(Snapshot {
                names: vec![Some("entry-0".to_string())],
            })
        });
        assert_counts(&job, 0, 0);
    }
    assert_eq!(tracking.outcome(Ok(false)).kind, OutcomeKind::Failed);
}

#[test]
fn empty_recycle_bin_final_absence_does_not_hide_real_item_or_operation_errors() {
    let (job, tracking, snapshot) = batch(2);
    tracking.record(0, true, E_ACCESSDENIED);
    snapshot.reconcile_with(&tracking, || Ok(Snapshot { names: Vec::new() }));
    assert_counts(&job, 1, 1);
    assert_eq!(tracking.outcome(Ok(false)).kind, OutcomeKind::Failed);

    let (job, tracking, snapshot) = batch(1);
    tracking.finish_result(E_ACCESSDENIED);
    snapshot.reconcile_with(&tracking, || Ok(Snapshot { names: Vec::new() }));
    assert_counts(&job, 1, 0);
    assert_eq!(tracking.outcome(Ok(false)).kind, OutcomeKind::Failed);
    assert_eq!(
        tracking
            .outcome(Err(Error::from_hresult(E_ACCESSDENIED)))
            .kind,
        OutcomeKind::Failed
    );
}

#[test]
fn empty_recycle_bin_final_absence_updates_counts_without_losing_cancellation() {
    let (job, tracking, snapshot) = batch(2);
    job.operation.progress.cancel();
    let calls = std::cell::Cell::new(0);
    snapshot.reconcile_with(&tracking, || {
        calls.set(calls.get() + 1);
        Ok(Snapshot {
            names: vec![Some("entry-1".to_string())],
        })
    });
    assert_eq!(calls.get(), 1);
    assert_counts(&job, 1, 0);
    assert_eq!(
        tracking.outcome(Err(Error::from_hresult(E_ABORT))).kind,
        OutcomeKind::Cancelled
    );
}

#[test]
fn empty_recycle_bin_final_absence_preserves_child_errors_after_a_root_callback() {
    let (job, tracking, snapshot) = batch(1);
    tracking.record(0, true, S_OK);
    tracking.record(0, false, E_ACCESSDENIED);
    tracking.record(0, false, E_ACCESSDENIED);
    snapshot.reconcile_with(&tracking, || Ok(Snapshot { names: Vec::new() }));
    assert_counts(&job, 0, 1);
    assert_eq!(tracking.outcome(Ok(false)).kind, OutcomeKind::Failed);
}

#[test]
fn empty_recycle_bin_snapshot_survives_removal_of_files_and_folders_in_a_disposable_tree() {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::SHCreateItemFromParsingName;

    let _com = recycle_bin::ComApartmentGuard::init_sta_best_effort();
    let directory = tempfile::tempdir().unwrap();
    let paths: Vec<_> = (0..8)
        .map(|index| directory.path().join(format!("entry-{index}")))
        .collect();
    for (index, path) in paths.iter().enumerate() {
        if index < 5 {
            std::fs::write(path, b"test").unwrap();
        } else {
            std::fs::create_dir(path).unwrap();
        }
    }
    let shell_item = |path: &std::path::Path| {
        let name: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        RecycleBinShellItem {
            item: unsafe { SHCreateItemFromParsingName(PCWSTR(name.as_ptr()), None) }.unwrap(),
        }
    };
    let original: Vec<_> = paths.iter().map(|path| shell_item(path)).collect();
    let snapshot = Snapshot::capture(&original);
    assert!(snapshot.names.iter().all(Option::is_some));
    let (job, tracking, _) = batch(8);
    for index in 0..5 {
        tracking.record(index, true, S_OK);
    }
    // No Shell deletion API and no access to the real Recycle Bin.
    for path in paths.iter().take(5) {
        std::fs::remove_file(path).unwrap();
    }
    std::fs::remove_dir(&paths[5]).unwrap();
    let remaining: Vec<_> = paths.iter().skip(6).map(|path| shell_item(path)).collect();
    snapshot.reconcile_with(&tracking, || Ok(Snapshot::capture(&remaining)));
    assert_counts(&job, 6, 0);
    for path in paths.iter().skip(6) {
        std::fs::remove_dir(path).unwrap();
    }
    snapshot.reconcile_with(&tracking, || Ok(Snapshot::capture(&[])));
    assert_counts(&job, 8, 0);
    assert_eq!(tracking.outcome(Ok(false)).kind, OutcomeKind::Completed);
}
