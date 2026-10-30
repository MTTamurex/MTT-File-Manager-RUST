use super::*;
use crate::app::disk_analysis_model::DiskAnalysisModel;
use mtt_search_protocol::{DiskAnalysisRecord, DiskAnalysisSnapshot};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;

mod measurements;
mod reader_tests;
mod selection_tests;

pub(super) fn fixture(
    contents: &[Vec<u8>],
) -> (tempfile::TempDir, Arc<DiskAnalysisModel>, PathResolver) {
    let dir = tempfile::tempdir().unwrap();
    let mut records = vec![DiskAnalysisRecord {
        frn: 5,
        parent_frn: 5,
        name: String::new(),
        size: 0,
        allocated_size: 0,
        is_dir: true,
        is_reparse: false,
    }];
    for (i, bytes) in contents.iter().enumerate() {
        let name = format!("file-{i}.bin");
        std::fs::write(dir.path().join(&name), bytes).unwrap();
        records.push(DiskAnalysisRecord {
            frn: 10 + i as u64,
            parent_frn: 5,
            name,
            size: bytes.len() as u64,
            allocated_size: bytes.len() as u64,
            is_dir: false,
            is_reparse: false,
        });
    }
    let model = Arc::new(DiskAnalysisModel::build(DiskAnalysisSnapshot {
        drive_letter: 'C',
        records,
    }));
    let resolver: PathResolver = {
        let model = model.clone();
        let root = dir.path().to_owned();
        Arc::new(move |idx| {
            model
                .nodes
                .get(idx as usize)
                .map(|node| root.join(&node.name))
        })
    };
    (dir, model, resolver)
}

pub(super) fn run(
    model: Arc<DiskAnalysisModel>,
    resolver: PathResolver,
    mode: DuplicateReadMode,
) -> (DuplicateReport, Vec<(DuplicatePhase, DuplicateProgress)>) {
    let (tx, rx) = channel();
    pipeline::run_scan(
        model.clone(),
        model.root,
        1,
        resolver,
        Arc::new(AtomicBool::new(false)),
        tx,
        mode.limits(),
    );
    let mut progress = Vec::new();
    for message in rx {
        match message {
            DuplicateMessage::Progress(phase, snapshot) => progress.push((phase, snapshot)),
            DuplicateMessage::Finished(
                DuplicateFinish::Complete(report) | DuplicateFinish::Partial(report),
            ) => return (*report, progress),
            other => panic!("unexpected finish: {other:?}"),
        }
    }
    panic!("missing scan result")
}

#[test]
fn grouping_keeps_only_multi_file_size_classes_sorted() {
    assert_eq!(
        group_by_size(vec![
            (7, 100),
            (3, 50),
            (9, 100),
            (1, 50),
            (2, 50),
            (5, 200)
        ]),
        vec![(50, vec![1, 2, 3]), (100, vec![7, 9])]
    );
}

#[test]
fn recoverable_math_is_conservative_and_hardlink_aware() {
    assert_eq!(
        compute_recoverable(&[([1, 1], 100), ([2, 2], 200), ([3, 3], 300)]),
        (300, false)
    );
    assert_eq!(
        compute_recoverable(&[([1, 1], 500), ([1, 1], 500)]),
        (0, true)
    );
    assert_eq!(
        compute_recoverable(&[([1, 1], 400), ([1, 1], 400), ([9, 9], 400)]),
        (400, true)
    );
    assert_eq!(compute_recoverable(&[([1, 1], 0), ([2, 2], 0)]), (0, false));
    assert_eq!(
        compute_recoverable(&[([1, 1], u64::MAX), ([2, 2], u64::MAX)]),
        (u64::MAX, false)
    );
}

#[test]
fn short_files_reuse_validated_complete_hashes_without_a_second_read() {
    for size in [4096, PARTIAL_BYTES as usize] {
        let (_dir, model, resolver) = fixture(&[vec![7; size], vec![7; size], vec![9; size]]);
        for mode in [DuplicateReadMode::Conservative, DuplicateReadMode::Parallel] {
            let (report, phases) = run(model.clone(), resolver.clone(), mode);
            assert_eq!(report.groups.len(), 1);
            assert_eq!(report.groups[0].members.len(), 2);
            assert_eq!(report.stats.hashed_bytes, (size * 3) as u64);
            assert_eq!(report.stats.full_candidates, 0);
            assert_eq!(report.stats.processed_files, 3);
            assert!(phases.iter().any(|p| p.0 == DuplicatePhase::Comparing));
        }
    }
}

#[test]
fn different_prefixes_skip_full_reads_but_equal_prefixes_do_not_prove_duplicates() {
    let size = PARTIAL_BYTES as usize * 2;
    let first = vec![7; size];
    let mut different_tail = first.clone();
    different_tail[size - 1] = 8;
    let (_dir, model, resolver) = fixture(&[first.clone(), first, different_tail, vec![9; size]]);
    for mode in [DuplicateReadMode::Conservative, DuplicateReadMode::Parallel] {
        let (report, phases) = run(model.clone(), resolver.clone(), mode);
        assert_eq!(report.groups.len(), 1);
        assert_eq!(report.groups[0].members.len(), 2);
        assert_eq!(report.stats.partial_rejected, 1);
        assert_eq!(report.stats.full_candidates, 3);
        assert_eq!(
            report.stats.hashed_bytes,
            4 * PARTIAL_BYTES + 3 * size as u64
        );
        assert_eq!(report.stats.processed_files, 4);
        for (_, progress) in phases {
            assert!(progress.phase_processed <= progress.phase_total);
            assert!(progress.processed_files <= progress.total_candidates);
        }
    }
}

#[test]
fn hardlinks_are_not_reported_as_independent_recoverable_copies() {
    let size = PARTIAL_BYTES as usize + 1;
    let (dir, model, resolver) = fixture(&[vec![7; size], vec![7; size], vec![7; size]]);
    // Rename our generated second file out of the way, then create a hardlink.
    std::fs::rename(dir.path().join("file-1.bin"), dir.path().join("backup.bin")).unwrap();
    std::fs::hard_link(dir.path().join("file-0.bin"), dir.path().join("file-1.bin")).unwrap();
    let (report, _) = run(model, resolver, DuplicateReadMode::Parallel);
    assert_eq!(report.groups.len(), 1);
    assert!(report.groups[0].has_hardlinks);
    assert_eq!(report.total_recoverable, size as u64);
}

#[test]
fn empty_subtree_reports_complete_empty_and_pre_cancelled_scan_is_cancelled() {
    let (_dir, model, resolver) = fixture(&[]);
    let (report, _) = run(
        model.clone(),
        resolver.clone(),
        DuplicateReadMode::Conservative,
    );
    assert!(report.groups.is_empty());
    let (tx, rx) = channel();
    pipeline::run_scan(
        model.clone(),
        model.root,
        1,
        resolver,
        Arc::new(AtomicBool::new(true)),
        tx,
        DuplicateReadMode::Conservative.limits(),
    );
    assert!(rx
        .into_iter()
        .any(|m| matches!(m, DuplicateMessage::Finished(DuplicateFinish::Cancelled))));
}

#[test]
fn minimum_size_requires_a_positive_integer() {
    let mut session = DuplicateSession::new();
    assert_eq!(session.min_size(), Some(DEFAULT_MIN_SIZE));
    for invalid in ["0", "invalid"] {
        session.min_size_text = invalid.into();
        assert_eq!(session.min_size(), None);
    }
}

#[test]
fn cancelling_session_drops_late_messages_in_both_read_phases() {
    for phase in [DuplicatePhase::Comparing, DuplicatePhase::Hashing] {
        let mut session = DuplicateSession::new();
        let (tx, rx) = channel();
        let flag = Arc::new(AtomicBool::new(false));
        session.phase = phase;
        session.cancel_flag = Some(flag.clone());
        session.receiver = Some(rx);
        session.cancel();
        assert!(flag.load(Ordering::Acquire));
        assert_eq!(session.phase, DuplicatePhase::Cancelled);
        assert!(session.receiver.is_none());
        assert!(tx
            .send(DuplicateMessage::Finished(DuplicateFinish::Failed(
                "late".into()
            )))
            .is_err());
        assert!(!session.poll());
    }
}

#[test]
fn inaccessible_candidates_produce_partial_results_not_an_unqualified_empty_report() {
    let (_dir, model, _) = fixture(&[vec![7; 4096], vec![7; 4096]]);
    let resolver: PathResolver = Arc::new(|_| None);
    let (tx, rx) = channel();
    pipeline::run_scan(
        model.clone(),
        model.root,
        1,
        resolver,
        Arc::new(AtomicBool::new(false)),
        tx,
        DuplicateReadMode::Parallel.limits(),
    );
    let finish = rx
        .into_iter()
        .find_map(|message| match message {
            DuplicateMessage::Finished(f) => Some(f),
            _ => None,
        })
        .unwrap();
    let DuplicateFinish::Partial(report) = finish else {
        panic!("expected incomplete result")
    };
    assert_eq!(report.stats.inaccessible, 2);
    assert_eq!(report.stats.processed_files, 2);
}

#[test]
fn cancellation_during_either_pass_has_a_single_cancelled_finish() {
    use std::sync::atomic::AtomicUsize;
    for cancel_on_open in [1, 3] {
        let (_dir, model, resolver) = fixture(&[
            vec![7; PARTIAL_BYTES as usize * 2],
            vec![7; PARTIAL_BYTES as usize * 2],
        ]);
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let opens = AtomicUsize::new(0);
        let resolver: PathResolver = Arc::new(move |idx| {
            if opens.fetch_add(1, Ordering::SeqCst) + 1 == cancel_on_open {
                flag.store(true, Ordering::Release);
            }
            resolver(idx)
        });
        let (tx, rx) = channel();
        pipeline::run_scan(
            model.clone(),
            model.root,
            1,
            resolver,
            cancel,
            tx,
            DuplicateReadMode::Conservative.limits(),
        );
        let finishes: Vec<_> = rx
            .into_iter()
            .filter_map(|m| match m {
                DuplicateMessage::Finished(f) => Some(f),
                _ => None,
            })
            .collect();
        assert_eq!(finishes.len(), 1);
        assert!(matches!(finishes[0], DuplicateFinish::Cancelled));
    }
}

#[test]
fn unexpected_reader_failure_finishes_the_session_instead_of_leaving_it_running() {
    let (_dir, model, _) = fixture(&[vec![7; 4096], vec![7; 4096]]);
    let resolver: PathResolver = Arc::new(|_| panic!("synthetic reader failure"));
    let (tx, rx) = channel();
    pipeline::run_scan(
        model.clone(),
        model.root,
        1,
        resolver,
        Arc::new(AtomicBool::new(false)),
        tx,
        DuplicateReadMode::Parallel.limits(),
    );
    let finishes: Vec<_> = rx
        .into_iter()
        .filter_map(|m| match m {
            DuplicateMessage::Finished(f) => Some(f),
            _ => None,
        })
        .collect();
    assert_eq!(finishes.len(), 1);
    assert!(matches!(finishes[0], DuplicateFinish::Failed(_)));
}

#[test]
fn hardlink_only_groups_have_no_recoverable_copies() {
    let (dir, model, resolver) = fixture(&[vec![7; 4096], vec![7; 4096]]);
    std::fs::rename(dir.path().join("file-1.bin"), dir.path().join("backup.bin")).unwrap();
    std::fs::hard_link(dir.path().join("file-0.bin"), dir.path().join("file-1.bin")).unwrap();
    let (report, _) = run(model, resolver, DuplicateReadMode::Conservative);
    assert!(report.groups.is_empty());
    assert_eq!(report.stats.hardlink_only_groups, 1);
    assert_eq!(report.total_recoverable, 0);
}
