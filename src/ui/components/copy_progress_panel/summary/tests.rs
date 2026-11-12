use super::*;
use crate::infrastructure::copy_engine::{CopyJob, CopyRuntime, Outcome};
use std::path::PathBuf;

fn job(runtime: &CopyRuntime, bytes: u64, total: u64, files: usize, total_files: usize) -> CopyJob {
    let job = runtime.start(
        vec![PathBuf::from("source/file.bin")],
        PathBuf::from("target"),
    );
    let progress = &job.operation.progress;
    progress.copying.store(true, Ordering::Relaxed);
    progress.bytes_copied.store(bytes, Ordering::Relaxed);
    progress.total_bytes.store(total, Ordering::Relaxed);
    progress.files_copied.store(files, Ordering::Relaxed);
    progress.total_files.store(total_files, Ordering::Relaxed);
    job
}

#[test]
fn header_weights_bytes_and_sums_counts_and_speeds_excluding_old_results() {
    let runtime = CopyRuntime::default();
    let _first = job(&runtime, 50, 100, 1, 2);
    let _second = job(&runtime, 300, 900, 3, 8);
    let old = job(&runtime, 9000, 9000, 100, 100);
    old.finish(Outcome {
        kind: OutcomeKind::Completed,
        message: None,
    });
    let mut panel = CopyPanelState::default();
    for (id, _) in runtime.jobs() {
        panel.metrics.entry(id).or_default().speed = (id + 1) as f64 * 1_048_576.0;
    }
    let summary = Summary::from_jobs(&runtime.jobs(), &panel).unwrap();
    assert_eq!(summary.fraction, Some(0.35));
    assert_eq!((summary.files, summary.total_files), (4, 10));
    assert_eq!(summary.speed, 3.0 * 1_048_576.0);
    assert_eq!(
        summary.label(),
        format!(
            "35% · {} · 3.0 MiB/s",
            t!("copy_engine.header_files", copied = 4, total = 10)
        )
    );
}

#[test]
fn paused_cancelled_and_preparing_operations_do_not_contribute_stale_speed() {
    let runtime = CopyRuntime::default();
    let paused = job(&runtime, 50, 100, 1, 2);
    paused.operation.progress.pause();
    let cancelled = job(&runtime, 25, 100, 0, 2);
    cancelled.operation.progress.cancel();
    let preparing = job(&runtime, 0, 0, 0, 0);
    preparing
        .operation
        .progress
        .copying
        .store(false, Ordering::Relaxed);
    let mut panel = CopyPanelState::default();
    for (id, _) in runtime.jobs() {
        panel.metrics.entry(id).or_default().speed = 1_048_576.0;
    }
    let summary = Summary::from_jobs(&runtime.jobs(), &panel).unwrap();
    assert!(summary.fraction.is_none());
    assert_eq!(summary.speed, 0.0);
    assert!(summary.label().starts_with('…'));
}

#[test]
fn empty_files_completed_results_and_failures_have_meaningful_progress() {
    let runtime = CopyRuntime::default();
    let empty = job(&runtime, 0, 0, 1, 2);
    let panel = CopyPanelState::default();
    assert_eq!(
        Summary::from_jobs(&runtime.jobs(), &panel)
            .unwrap()
            .fraction,
        Some(0.5)
    );
    empty.finish(Outcome {
        kind: OutcomeKind::Completed,
        message: None,
    });
    assert_eq!(
        Summary::from_jobs(&runtime.jobs(), &panel)
            .unwrap()
            .fraction,
        Some(1.0)
    );
    let failed = job(&runtime, 0, 0, 0, 0);
    failed.finish(Outcome {
        kind: OutcomeKind::Failed,
        message: None,
    });
    let summary = Summary::from_jobs(&runtime.jobs()[1..], &panel).unwrap();
    assert!(summary.fraction.is_none());
    assert!(summary.label().starts_with('—'));
    assert!(Summary::from_jobs(&[], &panel).is_none());
}

#[test]
fn counters_above_totals_and_large_sums_do_not_overflow_or_exceed_100_percent() {
    let runtime = CopyRuntime::default();
    let _first = job(&runtime, u64::MAX, u64::MAX, usize::MAX, usize::MAX);
    let _second = job(&runtime, u64::MAX, 100, 10, 2);
    let mut panel = CopyPanelState::default();
    for (id, _) in runtime.jobs() {
        panel.metrics.entry(id).or_default().speed = f64::NAN;
    }
    let summary = Summary::from_jobs(&runtime.jobs(), &panel).unwrap();
    assert_eq!(summary.fraction, Some(1.0));
    assert_eq!(summary.files, usize::MAX);
    assert_eq!(summary.speed, 0.0);
}
