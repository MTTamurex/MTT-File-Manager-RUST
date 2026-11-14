use super::*;

fn start(runtime: &Arc<CopyRuntime>) -> CopyJob {
    runtime.start(vec![PathBuf::from("source")], PathBuf::from("target"))
}

#[test]
fn closed_jobs_are_not_minimized_entries_and_are_collected_after_the_worker_finishes() {
    for kind in [
        OutcomeKind::Cancelled,
        OutcomeKind::Failed,
        OutcomeKind::Completed,
    ] {
        let runtime = Arc::new(CopyRuntime::default());
        let job = start(&runtime);
        let id = runtime.jobs()[0].0;
        job.operation.close();
        assert!(job.operation.progress.is_cancelled());
        assert!(runtime.jobs().is_empty());
        assert!(
            runtime.jobs.lock().unwrap().contains_key(&id),
            "Worker state must survive until completion"
        );
        let next = start(&runtime);
        assert_eq!(runtime.jobs().len(), 1);
        assert!(!next.operation.progress.is_cancelled());
        job.finish(Outcome {
            kind,
            message: None,
        });
        assert_eq!(job.operation.outcome().unwrap().kind, kind);
        assert_eq!(runtime.jobs().len(), 1);
        assert!(!runtime.jobs.lock().unwrap().contains_key(&id));
    }
}

#[test]
fn closing_a_finished_job_does_not_cancel_it_or_restore_its_result() {
    let runtime = Arc::new(CopyRuntime::default());
    let job = start(&runtime);
    job.finish(Outcome {
        kind: OutcomeKind::Completed,
        message: None,
    });
    job.operation.close();
    assert!(!job.operation.progress.is_cancelled());
    assert!(runtime.jobs().is_empty());
    assert_eq!(
        job.operation.outcome().unwrap().kind,
        OutcomeKind::Completed
    );
}

#[test]
fn jobs_have_independent_controls_and_retain_results_until_dismissed() {
    let runtime = Arc::new(CopyRuntime::default());
    let first = start(&runtime);
    let second = start(&runtime);
    first.operation.progress.pause();
    first.operation.progress.cancel();
    assert!(!second.operation.progress.is_cancelled());
    assert_eq!(
        second.operation.progress.pause_state(),
        mtt_copy_engine::PauseState::Running
    );
    let first_id = runtime.jobs()[0].0;
    runtime.dismiss(first_id);
    assert_eq!(runtime.jobs().len(), 2);
    first.finish(Outcome {
        kind: OutcomeKind::Cancelled,
        message: None,
    });
    drop(first);
    assert_eq!(runtime.jobs().len(), 2);
    runtime.dismiss(first_id);
    assert_eq!(runtime.jobs().len(), 1);
    second.finish(Outcome {
        kind: OutcomeKind::Completed,
        message: None,
    });
    drop(second);
    let id = runtime.jobs()[0].0;
    runtime.dismiss(id);
    assert!(runtime.jobs().is_empty());
}

#[test]
fn unfinished_guard_reports_failure_instead_of_disappearing() {
    let runtime = Arc::new(CopyRuntime::default());
    drop(start(&runtime));
    let operation = &runtime.jobs()[0].1;
    let outcome = operation.outcome().unwrap();
    assert_eq!(outcome.kind, OutcomeKind::Failed);
    assert!(outcome.message.is_some());
}

#[test]
fn elapsed_excludes_pauses_and_is_retained_for_every_terminal_outcome() {
    let runtime = Arc::new(CopyRuntime::default());
    for kind in [
        OutcomeKind::Completed,
        OutcomeKind::Cancelled,
        OutcomeKind::Failed,
    ] {
        let job = start(&runtime);
        let initial = job.operation.elapsed();
        job.operation.progress.pause();
        let paused = job.operation.elapsed();
        assert!(paused >= initial);
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(job.operation.elapsed(), paused);
        job.finish(Outcome {
            kind,
            message: None,
        });
        assert_eq!(job.operation.elapsed(), paused);
        job.operation.progress.resume();
        let operation = job.operation.clone();
        drop(job);
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(operation.elapsed(), paused);
        assert_eq!(operation.outcome().unwrap().kind, kind);
    }
}

#[test]
fn interrupted_job_also_retains_its_elapsed_time() {
    let runtime = Arc::new(CopyRuntime::default());
    let job = start(&runtime);
    job.operation.progress.pause();
    let elapsed = job.operation.elapsed();
    let operation = job.operation.clone();
    drop(job);
    operation.progress.resume();
    std::thread::sleep(Duration::from_millis(10));
    assert_eq!(operation.elapsed(), elapsed);
    assert_eq!(operation.outcome().unwrap().kind, OutcomeKind::Failed);
}
