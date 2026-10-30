use super::super::reader::FileStamp;
use super::*;

#[test]
fn stage_workers_obey_limits_process_each_job_once_and_aggregate_bytes() {
    for slots in [1, 2, 4] {
        let jobs: Vec<_> = (0..20)
            .map(|idx| ReadJob {
                idx,
                size: 100,
                previous: None,
            })
            .collect();
        let active = AtomicUsize::new(0);
        let maximum = AtomicUsize::new(0);
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut progress = DuplicateProgress::default();
        let cancel = AtomicBool::new(false);
        let hashes = run_stage(
            (jobs.len(), |i| jobs[i]),
            slots,
            DuplicatePhase::Hashing,
            &cancel,
            &mut progress,
            &tx,
            |job, on_read| {
                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                maximum.fetch_max(count, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(2));
                on_read(job.size);
                active.fetch_sub(1, Ordering::SeqCst);
                ReadOutcome::Hashed(ValidatedHash {
                    digest: [0; 32],
                    stamp: FileStamp {
                        identity: [1, job.idx as u64],
                        size: job.size,
                        creation: 0,
                        modified: 0,
                    },
                })
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(hashes.len(), jobs.len());
        assert!(maximum.load(Ordering::SeqCst) <= slots);
        assert_eq!(progress.phase_processed, 20);
        assert_eq!(progress.processed_files, 20);
        assert_eq!(progress.hashed_bytes, 2000);
    }
}

#[test]
fn cancellation_stops_scheduling_and_keeps_interrupted_read_bytes() {
    let jobs: Vec<_> = (0..20)
        .map(|idx| ReadJob {
            idx,
            size: 100,
            previous: None,
        })
        .collect();
    let (tx, _rx) = std::sync::mpsc::channel();
    let mut progress = DuplicateProgress::default();
    let cancel = AtomicBool::new(false);
    let result = run_stage(
        (jobs.len(), |i| jobs[i]),
        1,
        DuplicatePhase::Comparing,
        &cancel,
        &mut progress,
        &tx,
        |_, on_read| {
            on_read(23);
            cancel.store(true, Ordering::Release);
            ReadOutcome::Cancelled
        },
    )
    .unwrap();
    assert!(result.is_none());
    assert_eq!(progress.hashed_bytes, 23);
    assert_eq!(progress.phase_processed, 1);
}
