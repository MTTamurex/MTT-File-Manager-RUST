use super::*;
use crate::app::disk_analysis_duplicates::tests::{fixture, run};

fn request() -> (tempfile::TempDir, Request) {
    let (dir, model, resolver) = fixture(&[vec![7; 4096], vec![7; 4096], vec![7; 4096]]);
    let (report, _) = run(model, resolver.clone(), DuplicateReadMode::Conservative);
    let report = Arc::new(report);
    let target = report.groups[0].members[0].clone();
    let survivor = report.groups[0].members[1].clone();
    let request = Request {
        target_path: resolver(target.idx).unwrap(),
        survivor_path: resolver(survivor.idx).unwrap(),
        report,
        target,
        survivor,
    };
    (dir, request)
}

#[test]
fn identical_copies_dispatch_only_the_explicit_target_to_the_existing_shell_boundary() {
    let (dir, request) = request();
    let original = std::fs::read(&request.survivor_path).unwrap();
    let mut called = false;
    let result = validation::execute(&request, &AtomicBool::new(false), |path| {
        assert_eq!(path, request.target_path);
        called = true;
        // Emulate successful Shell removal from the original location without
        // running Windows deletion dialogs or changing anything outside fixtures.
        std::fs::rename(path, dir.path().join("removed.bin")).unwrap();
        true
    });
    assert!(called);
    assert_eq!(result, (DeleteStatus::Removed, true));
    assert_eq!(std::fs::read(&request.survivor_path).unwrap(), original);
}

#[test]
fn shell_cancel_has_no_filesystem_deletion_fallback() {
    let (_dir, request) = request();
    assert_eq!(
        validation::execute(&request, &AtomicBool::new(false), |_| false),
        (DeleteStatus::CancelledOrFailed, true)
    );
    assert!(request.target_path.exists());
    assert!(request.survivor_path.exists());
}

#[test]
fn missing_survivor_and_replaced_target_never_reach_shell() {
    let (dir, request) = request();
    std::fs::rename(&request.survivor_path, dir.path().join("previous.bin")).unwrap();
    assert_eq!(
        validation::execute(&request, &AtomicBool::new(false), |_| panic!(
            "unsafe dispatch"
        )),
        (DeleteStatus::Unsafe, false)
    );
    let (dir, request) = self::request();
    std::fs::rename(&request.target_path, dir.path().join("previous.bin")).unwrap();
    std::fs::write(&request.target_path, vec![7; 4096]).unwrap();
    assert_eq!(
        validation::execute(&request, &AtomicBool::new(false), |_| panic!(
            "replacement dispatched"
        )),
        (DeleteStatus::Unsafe, false)
    );
}

#[test]
fn content_difference_is_detected_even_with_restored_size_and_write_time() {
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
    assert_eq!(
        validation::execute(&request, &AtomicBool::new(false), |_| panic!(
            "different content dispatched"
        )),
        (DeleteStatus::Unsafe, false)
    );
}

#[test]
fn same_file_same_path_and_shared_physical_identity_are_rejected() {
    let (_dir, request) = request();
    for case in 0..3 {
        let mut invalid = request.clone();
        match case {
            0 => invalid.survivor.stamp.identity = invalid.target.stamp.identity,
            1 => invalid.survivor.idx = invalid.target.idx,
            _ => invalid.survivor_path = invalid.target_path.clone(),
        }
        assert_eq!(
            validation::execute(&invalid, &AtomicBool::new(false), |_| panic!(
                "aliased file dispatched"
            )),
            (DeleteStatus::Unsafe, false)
        );
    }
}

#[test]
fn cancelled_validation_never_dispatches() {
    let (_dir, request) = request();
    assert_eq!(
        validation::execute(&request, &AtomicBool::new(true), |_| panic!(
            "cancelled dispatch"
        )),
        (DeleteStatus::CancelledOrFailed, false)
    );
}

#[cfg(windows)]
#[test]
fn survivor_cannot_be_written_or_renamed_during_the_native_call() {
    let (dir, request) = request();
    let mut called = false;
    validation::execute(&request, &AtomicBool::new(false), |_| {
        called = true;
        assert!(std::fs::OpenOptions::new()
            .write(true)
            .open(&request.survivor_path)
            .is_err());
        assert!(std::fs::OpenOptions::new()
            .write(true)
            .open(&request.target_path)
            .is_err());
        assert!(std::fs::rename(&request.survivor_path, dir.path().join("moved.bin")).is_err());
        false
    });
    assert!(called);
}

#[test]
fn successful_removal_updates_report_and_never_offers_the_last_copy() {
    let (_dir, request) = request();
    let mut session = DuplicateSession::new();
    session.report = Some(request.report.clone());
    let removed = request.target.idx;
    apply_completion(
        &mut session,
        Completion {
            request: request.clone(),
            status: DeleteStatus::Removed,
            shell_called: true,
        },
    );
    let report = session.report.as_ref().unwrap();
    assert!(session.snapshot_stale);
    assert_eq!(report.groups[0].members.len(), 2);
    assert!(!report.groups[0].members.iter().any(|m| m.idx == removed));
    let mut report = report.as_ref().clone();
    let next = report.groups[0].members[0].idx;
    report::remove_member(&mut report, next);
    assert!(report.groups.is_empty());
    assert_eq!(report.total_recoverable, 0);
}

#[test]
fn completion_for_an_old_report_cannot_mutate_a_replacement() {
    let (_dir, request) = request();
    let mut session = DuplicateSession::new();
    session.report = Some(Arc::new(request.report.as_ref().clone()));
    apply_completion(
        &mut session,
        Completion {
            request,
            status: DeleteStatus::Removed,
            shell_called: true,
        },
    );
    assert_eq!(session.report.as_ref().unwrap().groups[0].members.len(), 3);
    assert!(session.snapshot_stale);
}

#[test]
fn cancelling_an_idle_operation_does_not_start_deletion() {
    let mut deletion = DuplicateDeletion::default();
    deletion.cancel();
    assert!(!deletion.is_open());
}

#[test]
fn requests_require_two_verified_visible_copies_from_the_same_report() {
    let (_dir, request) = request();
    let (_, model, _) = fixture(&[vec![7; 4096], vec![7; 4096], vec![7; 4096]]);
    let mut review = DuplicateReview::default();
    review.open_group(&request.report, 0);
    assert!(
        DuplicateDeletion::prepare(&request.report, &review, &model, request.target.idx).is_none()
    );
    for entry in review.entries.iter_mut().flatten() {
        entry.info = Some(super::super::review::CopyInfo {
            status: CopyStatus::Current,
            size: Some(4096),
            created: None,
            modified: None,
            preview_status: PreviewStatus::NotImage,
            image: None,
        });
    }
    let replacement = Arc::new(request.report.as_ref().clone());
    assert!(
        DuplicateDeletion::prepare(&replacement, &review, &model, request.target.idx).is_none()
    );
    let prepared =
        DuplicateDeletion::prepare(&request.report, &review, &model, request.target.idx).unwrap();
    assert_eq!(prepared.target.idx, request.target.idx);
    assert_eq!(prepared.survivor.idx, request.survivor.idx);
}

#[test]
fn asynchronous_validation_failure_never_calls_native_deletion() {
    let (dir, mut request) = request();
    request.survivor_path = dir.path().join("missing.bin");
    let mut deletion = DuplicateDeletion::default();
    deletion.launch(request, 0);
    assert!(deletion.is_running());
    let completion = deletion
        .receiver
        .as_ref()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert_eq!(completion.status, DeleteStatus::Unsafe);
    assert!(!completion.shell_called);
}

#[cfg(windows)]
#[test]
fn offline_target_never_reaches_shell() {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{
            SetFileAttributesW, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_OFFLINE,
        },
    };
    let (_dir, request) = request();
    let wide: Vec<_> = request
        .target_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        SetFileAttributesW(PCWSTR(wide.as_ptr()), FILE_ATTRIBUTE_OFFLINE).unwrap();
    }
    let outcome = validation::execute(&request, &AtomicBool::new(false), |_| {
        panic!("offline file dispatched")
    });
    unsafe {
        SetFileAttributesW(PCWSTR(wide.as_ptr()), FILE_ATTRIBUTE_NORMAL).unwrap();
    }
    assert_eq!(outcome, (DeleteStatus::Unsafe, false));
}
