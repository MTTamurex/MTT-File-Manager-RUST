use super::*;
use crate::infrastructure::copy_engine::CopyRuntime;
use std::sync::mpsc;
use windows::Win32::Foundation::E_ACCESSDENIED;

#[test]
fn observed_roots_advance_progress_before_the_operation_finishes_without_shell_callbacks() {
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete(Vec::new(), true);
    let tracking = Arc::new(Tracking::new(job.operation.clone(), 5));
    let (updates, pending) = mpsc::channel::<usize>();
    let (acknowledge, observed) = mpsc::channel();
    let observer_tracking = tracking.clone();
    let observer = LiveProgress::start_with(move |_| {
        if let Ok(removed) = pending.try_recv() {
            for index in 0..5 {
                observer_tracking.reconcile_item(index, index >= removed);
            }
            acknowledge.send(()).unwrap();
        }
    })
    .unwrap();
    let progress = job.operation.deletion.as_ref().unwrap();
    for removed in [1, 1, 3, 5] {
        updates.send(removed).unwrap();
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(job.operation.outcome().is_none());
        assert_eq!(
            progress.permanently_removed.load(Ordering::Relaxed),
            removed as u64
        );
        assert_eq!(progress.removed.load(Ordering::Relaxed), removed as u64);
    }
    drop(observer);
}

#[test]
fn observer_shutdown_interrupts_an_observation_and_joins_before_final_counts() {
    let (started, waiting) = mpsc::channel();
    let (finished, joined) = mpsc::channel();
    let observer = LiveProgress::start_with(move |stopped| {
        started.send(()).unwrap();
        while !stopped.load(Ordering::Acquire) {
            thread::yield_now();
        }
        finished.send(()).unwrap();
    })
    .unwrap();
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(observer);
    joined.try_recv().unwrap();
}

#[test]
fn observation_after_cancellation_preserves_real_failures_and_partial_removals() {
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete(Vec::new(), true);
    let tracking = Arc::new(Tracking::new(job.operation.clone(), 3));
    tracking.record(1, false, E_ACCESSDENIED);
    job.operation.progress.cancel();
    let (acknowledge, observed) = mpsc::channel();
    let observer = LiveProgress::start_with(move |_| {
        tracking.reconcile_item(0, false);
        tracking.reconcile_item(1, false);
        tracking.reconcile_item(2, true);
        let _ = acknowledge.send(());
    })
    .unwrap();
    observed.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(observer);
    let progress = job.operation.deletion.as_ref().unwrap();
    assert_eq!(progress.permanently_removed.load(Ordering::Relaxed), 1);
    assert_eq!(progress.failures.load(Ordering::Relaxed), 1);
    assert!(job.operation.progress.is_cancelled());
}
