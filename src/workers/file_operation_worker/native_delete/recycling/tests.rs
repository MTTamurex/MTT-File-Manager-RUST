use super::*;
use crate::infrastructure::copy_engine::CopyRuntime;

#[test]
fn cancellation_before_recycling_never_submits_items_or_requests_fallback() {
    let paths = vec![PathBuf::from("source/file.txt")];
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete(paths.clone(), false);
    job.operation.progress.cancel();

    let completed = run_with_recycler(
        &paths,
        &job.operation,
        Arc::new(|_| panic!("cancelled operation must not report removals")),
        &|_, _, _| panic!("cancelled operation must not request permanent deletion"),
        |_, _, _| panic!("cancelled operation must not submit items to Shell"),
    );

    assert!(!completed);
    let deletion = job.operation.deletion.as_ref().unwrap();
    assert_eq!(deletion.removed.load(Ordering::Relaxed), 0);
    assert_eq!(deletion.failures.load(Ordering::Relaxed), 0);
}

#[test]
fn unavailable_recycling_never_deletes_without_a_separate_permanent_answer() {
    for allow in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("file.txt");
        std::fs::write(&path, b"synthetic").unwrap();
        let runtime = CopyRuntime::default();
        let job = runtime.start_delete(vec![path.clone()], false);
        let completed = run_with_recycler(
            std::slice::from_ref(&path),
            &job.operation,
            Arc::new(|_| {}),
            &|_, kind, paths| {
                assert_eq!(kind, Kind::RecycleUnavailable);
                assert_eq!(paths, std::slice::from_ref(&path));
                assert!(path.exists());
                allow
            },
            |paths, _, _| {
                assert_eq!(paths, std::slice::from_ref(&path));
                Ok(recycle::BatchOutcome {
                    unavailable: vec![path.clone()],
                    ..Default::default()
                })
            },
        );
        assert_eq!(completed, allow);
        assert_eq!(path.exists(), !allow);
        assert_eq!(
            job.operation
                .deletion
                .as_ref()
                .unwrap()
                .used_permanent_fallback
                .load(Ordering::Relaxed),
            allow
        );
        temp.close().unwrap();
    }
}

#[test]
fn fallback_is_limited_to_the_unavailable_root_and_declining_preserves_pending_items() {
    let temp = tempfile::tempdir().unwrap();
    let paths: Vec<_> = (0..3)
        .map(|index| temp.path().join(format!("file-{index}.txt")))
        .collect();
    for path in &paths {
        std::fs::write(path, b"synthetic").unwrap();
    }
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete(paths.clone(), false);
    let completed = run_with_recycler(
        &paths,
        &job.operation,
        Arc::new(|_| {}),
        &|_, kind, pending| {
            assert_eq!(kind, Kind::RecycleUnavailable);
            assert_eq!(pending, &paths[1..2]);
            false
        },
        |batch, _, _| {
            assert_eq!(batch, paths.as_slice());
            std::fs::remove_file(&paths[0]).unwrap();
            Ok(recycle::BatchOutcome {
                unavailable: vec![paths[1].clone()],
                pending: vec![paths[2].clone()],
                ..Default::default()
            })
        },
    );
    assert!(!completed);
    assert!(!paths[0].exists());
    assert!(paths[1].exists());
    assert!(paths[2].exists());
    temp.close().unwrap();
}

#[test]
fn accepted_fallback_processes_pending_batch_and_cancellation_stops_remaining_items() {
    use crate::infrastructure::delete_engine::confirmation::{ask, tests::wait_for_request};

    for cancel_after_first_recycle in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let paths: Vec<_> = (0..3)
            .map(|index| temp.path().join(format!("file-{index}.txt")))
            .collect();
        for path in &paths {
            std::fs::write(path, b"synthetic").unwrap();
        }
        let recycled = temp.path().join("recycled");
        std::fs::create_dir(&recycled).unwrap();
        let runtime = CopyRuntime::default();
        let job = runtime.start_delete(paths.clone(), false);
        let operation = job.operation.clone();
        let control = operation.clone();
        let cancellation_trigger = paths[1].clone();
        let observer: RemovalObserver = Arc::new(move |removed_path| {
            if cancel_after_first_recycle && removed_path == cancellation_trigger {
                control.progress.cancel();
            }
        });
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker_paths = paths.clone();
        let worker_recycled = recycled.clone();
        let worker = std::thread::spawn(move || {
            let mut first_call = true;
            let completed = run_with_recycler(
                &worker_paths,
                &operation,
                observer.clone(),
                &ask,
                |batch, operation, observer| {
                    if first_call {
                        first_call = false;
                        return Ok(recycle::BatchOutcome {
                            unavailable: vec![worker_paths[0].clone()],
                            pending: worker_paths[1..].to_vec(),
                            ..Default::default()
                        });
                    }
                    assert!(
                        !operation
                            .deletion
                            .as_ref()
                            .unwrap()
                            .permanent_phase
                            .load(Ordering::Relaxed),
                        "recycling must not inherit the permanent-fallback UI phase"
                    );

                    let mut pending = Vec::new();
                    for (index, path) in batch.iter().enumerate() {
                        if operation.progress.is_cancelled() {
                            pending.extend(batch[index..].iter().cloned());
                            break;
                        }
                        std::fs::rename(path, worker_recycled.join(path.file_name().unwrap()))
                            .unwrap();
                        operation.deletion.as_ref().unwrap().recycled();
                        observer(path);
                        if operation.progress.is_cancelled() {
                            pending.extend(batch[index + 1..].iter().cloned());
                            break;
                        }
                    }
                    Ok(recycle::BatchOutcome {
                        pending,
                        cancelled: operation.progress.is_cancelled(),
                        ..Default::default()
                    })
                },
            );
            let _ = sender.send(completed);
        });

        let request = wait_for_request(&job.operation);
        assert_eq!(request.paths, paths[..1]);
        assert!(job
            .operation
            .deletion
            .as_ref()
            .unwrap()
            .confirmation
            .respond_with_scope(request.id, true, true));
        let completed = receiver.recv_timeout(std::time::Duration::from_secs(5));
        if completed.is_err() {
            job.operation.progress.cancel();
        }
        worker.join().unwrap();

        assert_eq!(completed, Ok(!cancel_after_first_recycle));
        assert!(!paths[0].exists(), "the confirmed fallback root is removed");
        assert!(
            !paths[1].exists(),
            "the first pending item was recycled (cancel={cancel_after_first_recycle})"
        );
        assert_eq!(
            paths[2].exists(),
            cancel_after_first_recycle,
            "cancellation preserves items not yet processed"
        );
        assert!(
            recycled.join(paths[1].file_name().unwrap()).exists(),
            "the successfully recycled item is retained in the test bin"
        );
        assert_eq!(
            recycled.join(paths[2].file_name().unwrap()).exists(),
            !cancel_after_first_recycle
        );
        temp.close().unwrap();
    }
}

#[test]
fn large_selections_are_submitted_in_bounded_batches() {
    let paths: Vec<_> = (0..(RECYCLE_BATCH_SIZE * 2 + 1))
        .map(|index| PathBuf::from(format!("synthetic/file-{index}.txt")))
        .collect();
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete(paths.clone(), false);
    let batch_sizes = std::sync::Mutex::new(Vec::new());
    let completed = run_with_recycler(
        &paths,
        &job.operation,
        Arc::new(|_| {}),
        &|_, _, _| true,
        |batch, _, _| {
            batch_sizes.lock().unwrap().push(batch.len());
            Ok(recycle::BatchOutcome::default())
        },
    );

    assert!(completed);
    assert_eq!(
        *batch_sizes.lock().unwrap(),
        [RECYCLE_BATCH_SIZE, RECYCLE_BATCH_SIZE, 1]
    );
}
