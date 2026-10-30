use super::*;
use crate::app::disk_analysis_duplicates::tests::{fixture, run};

#[test]
fn failed_refresh_marks_pending_copies_inaccessible() {
    let (_dir, model, report, resolver) = confirmed();
    let mut review = DuplicateReview::default();
    review.open_group(&report, 0);
    load_slots(&mut review, &report, &resolver);
    review.refresh();
    review.worker_failed = true;
    review.queue_visible(&model);
    assert!(!review.is_pending());
    assert!(review
        .entries
        .iter()
        .flatten()
        .all(|e| e.info.as_ref().unwrap().status == CopyStatus::Inaccessible));
}

#[test]
fn disconnected_refresh_marks_pending_copies_inaccessible() {
    let (_dir, _model, report, resolver) = confirmed();
    let mut review = DuplicateReview::default();
    review.open_group(&report, 0);
    load_slots(&mut review, &report, &resolver);
    review.refresh();
    let (requests, _jobs) = std::sync::mpsc::sync_channel(2);
    let (results, responses) = std::sync::mpsc::sync_channel(2);
    review.worker = Some(worker::Worker {
        requests,
        responses,
    });
    drop(results);
    assert!(review.poll());
    assert!(!review.is_pending());
}

#[test]
fn background_worker_returns_validated_metadata() {
    let (_dir, _model, report, resolver) = confirmed();
    let generation = Arc::new(AtomicU64::new(4));
    let worker = worker::Worker::spawn(generation).unwrap();
    let member = report.groups[0].members[0].clone();
    worker
        .requests
        .send(worker::Request {
            generation: 4,
            slot: 0,
            path: resolver(member.idx).unwrap(),
            member,
        })
        .unwrap();
    let response = worker
        .responses
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert_eq!(response.generation, 4);
    assert_eq!(response.info.status, CopyStatus::Current);
}

#[test]
fn valid_and_invalid_worker_responses_update_current_status() {
    let (_dir, _model, report, resolver) = confirmed();
    let mut review = DuplicateReview::default();
    review.open_group(&report, 0);
    let idx = review.entries[0].as_ref().unwrap().idx;
    let generation = review.generation.load(Ordering::Acquire);
    let member = &report.groups[0].members[0];
    let (requests, _jobs) = std::sync::mpsc::sync_channel(2);
    let (results, responses) = std::sync::mpsc::sync_channel(2);
    review.worker = Some(worker::Worker {
        requests,
        responses,
    });
    results
        .send(worker::Response {
            generation,
            slot: 0,
            idx,
            info: loader::load(&resolver(idx).unwrap(), member, || false),
        })
        .unwrap();
    assert!(review.poll());
    assert!(review.current(idx));
    results
        .send(worker::Response {
            generation,
            slot: 0,
            idx,
            info: CopyInfo::unavailable(CopyStatus::Changed),
        })
        .unwrap();
    assert!(review.poll());
    assert!(!review.current(idx));
}

#[cfg(windows)]
#[test]
fn offline_copy_is_not_loaded_for_review() {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{
            SetFileAttributesW, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_OFFLINE,
        },
    };
    let (_dir, _model, report, resolver) = confirmed();
    let member = &report.groups[0].members[0];
    let path = resolver(member.idx).unwrap();
    let wide: Vec<_> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        SetFileAttributesW(PCWSTR(wide.as_ptr()), FILE_ATTRIBUTE_OFFLINE).unwrap();
    }
    let info = loader::load(&path, member, || false);
    unsafe {
        SetFileAttributesW(PCWSTR(wide.as_ptr()), FILE_ATTRIBUTE_NORMAL).unwrap();
    }
    assert_eq!(info.status, CopyStatus::Unavailable);
    assert!(info.image.is_none());
    assert!(info.size.is_none());
}

fn confirmed() -> (
    tempfile::TempDir,
    Arc<crate::app::disk_analysis_model::DiskAnalysisModel>,
    Arc<DuplicateReport>,
    PathResolver,
) {
    let (dir, model, resolver) = fixture(&[vec![7; 4096], vec![7; 4096], vec![7; 4096]]);
    let (report, _) = run(
        model.clone(),
        resolver.clone(),
        DuplicateReadMode::Conservative,
    );
    (dir, model, Arc::new(report), resolver)
}

fn load_slots(review: &mut DuplicateReview, report: &DuplicateReport, resolver: &PathResolver) {
    for entry in review.entries.iter_mut().flatten() {
        let member = report.groups[0]
            .members
            .iter()
            .find(|m| m.idx == entry.idx)
            .unwrap();
        entry.info = Some(loader::load(&resolver(entry.idx).unwrap(), member, || {
            false
        }));
    }
}

#[test]
fn switching_visible_copies_is_bounded_and_distinct() {
    let (_dir, _model, report, resolver) = confirmed();
    let mut review = DuplicateReview::default();
    review.open_group(&report, 0);
    load_slots(&mut review, &report, &resolver);
    let first = report.groups[0].members[0].idx;
    let second = report.groups[0].members[1].idx;
    assert!(review.select_copy(0, second));
    assert_eq!(review.entries[0].as_ref().unwrap().idx, second);
    assert_eq!(review.entries[1].as_ref().unwrap().idx, first);
    assert!(!review.select_copy(0, u32::MAX));
    review.close();
    assert!(review.entries.iter().all(Option::is_none));
}

#[test]
fn report_replacement_clears_preview_state_even_with_reused_indices() {
    let (_dir, _model, report, resolver) = confirmed();
    let mut review = DuplicateReview::default();
    review.open_group(&report, 0);
    load_slots(&mut review, &report, &resolver);
    review.bind_report(&report);
    assert_eq!(review.active_group, Some(0));
    review.bind_report(&Arc::new(report.as_ref().clone()));
    assert_eq!(review.active_group, None);
    assert!(review.entries.iter().all(Option::is_none));
}

#[test]
fn missing_replaced_and_modified_files_are_not_current() {
    let (dir, _model, report, resolver) = confirmed();
    let member = &report.groups[0].members[0];
    let path = resolver(member.idx).unwrap();
    assert_eq!(
        loader::load(&path, member, || false).status,
        CopyStatus::Current
    );
    std::fs::rename(&path, dir.path().join("previous.bin")).unwrap();
    assert_eq!(
        loader::load(&path, member, || false).status,
        CopyStatus::Missing
    );
    std::fs::write(&path, vec![7; 4096]).unwrap();
    assert_eq!(
        loader::load(&path, member, || false).status,
        CopyStatus::Changed
    );
    let member = &report.groups[0].members[1];
    let path = resolver(member.idx).unwrap();
    let writer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    writer.set_len(4097).unwrap();
    drop(writer);
    assert_eq!(
        loader::load(&path, member, || false).status,
        CopyStatus::Changed
    );
}

#[test]
fn stale_worker_responses_are_not_applied_to_a_new_pair() {
    let (_dir, _model, report, _resolver) = confirmed();
    let mut review = DuplicateReview::default();
    review.open_group(&report, 0);
    let generation = review.generation.load(Ordering::Acquire);
    let idx = review.entries[0].as_ref().unwrap().idx;
    let (requests, _jobs) = std::sync::mpsc::sync_channel(2);
    let (results, responses) = std::sync::mpsc::sync_channel(2);
    review.worker = Some(worker::Worker {
        requests,
        responses,
    });
    review.refresh();
    results
        .send(worker::Response {
            generation,
            slot: 0,
            idx,
            info: CopyInfo::unavailable(CopyStatus::Changed),
        })
        .unwrap();
    assert!(!review.poll());
    assert!(review.entries[0].as_ref().unwrap().info.is_none());
}

#[test]
fn reading_comparison_metadata_does_not_modify_content_or_write_time() {
    let (_dir, _model, report, resolver) = confirmed();
    for member in &report.groups[0].members {
        let path = resolver(member.idx).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let info = loader::load(&path, member, || false);
        assert_eq!(info.status, CopyStatus::Current);
        assert_eq!(info.preview_status, PreviewStatus::NotImage);
        assert!(info.modified.is_some());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified
        );
    }
}
