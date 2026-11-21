use super::*;
use crate::infrastructure::drive_indexing::DriveIndexingAvailability;
use mtt_search_protocol::DriveIndexingEntry;
use std::collections::VecDeque;

fn config(enabled: bool) -> DriveIndexingSnapshot {
    DriveIndexingSnapshot::new(
        vec![DriveIndexingEntry {
            drive_letter: 'R',
            enabled,
        }],
        true,
        vec![],
    )
    .unwrap()
}

struct Fake {
    reads: VecDeque<Result<DriveIndexingSnapshot, DriveIndexingError>>,
    write: Result<(), DriveIndexingError>,
    writes: Vec<(char, bool)>,
}

impl Backend for Fake {
    fn get(&mut self) -> Result<DriveIndexingSnapshot, DriveIndexingError> {
        self.reads.pop_front().expect("unexpected service GET")
    }
    fn set(&mut self, letter: char, enabled: bool) -> Result<(), DriveIndexingError> {
        self.writes.push((letter, enabled));
        self.write.clone()
    }
}

fn core() -> WorkerCore {
    let policy = SharedDriveIndexing::from_cache(None);
    policy.confirm(config(true));
    WorkerCore {
        policy,
        status: Arc::new(RwLock::new(DriveIndexingWorkerStatus::default())),
    }
}

fn change(letter: char) -> DriveIndexingChange {
    DriveIndexingChange {
        request_id: letter as u64,
        drive_letter: letter,
        enabled: false,
    }
}

#[test]
fn acknowledgement_alone_does_not_overwrite_confirmed_cache() {
    let mut core = core();
    let cache = core.policy.snapshot().cache_json();
    let mut backend = Fake {
        reads: VecDeque::from([
            Err(DriveIndexingError::transport("read timed out")),
            Ok(config(false)),
        ]),
        write: Ok(()),
        writes: vec![],
    };
    core.set(change('R'), &mut backend);
    assert_eq!(core.policy.snapshot().cache_json(), cache);
    assert_eq!(
        core.status.read().operations[&'R'].outcome,
        DriveIndexingWriteOutcome::Pending
    );
    core.refresh(&mut backend);
    assert!(!core.policy.is_enabled('R'));
    assert_eq!(
        core.status.read().operations[&'R'].outcome,
        DriveIndexingWriteOutcome::Confirmed { enabled: false }
    );
    assert_eq!(backend.writes.len(), 1);
}

#[test]
fn a_lost_write_reply_is_reconciled_without_replaying_the_write() {
    let mut core = core();
    let mut backend = Fake {
        reads: VecDeque::from([
            Err(DriveIndexingError::transport("offline")),
            Ok(config(false)),
        ]),
        write: Err(DriveIndexingError::transport("acknowledgement lost")),
        writes: vec![],
    };
    core.set(change('R'), &mut backend);
    assert!(core.policy.is_enabled('R'));
    core.refresh(&mut backend);
    assert!(!core.policy.is_enabled('R'));
    assert_eq!(backend.writes, vec![('R', false)]);
}

#[test]
fn reconciliation_can_confirm_a_different_value_from_another_instance() {
    let mut core = core();
    let mut backend = Fake {
        reads: VecDeque::from([Ok(config(true))]),
        write: Err(DriveIndexingError::transport("timeout")),
        writes: vec![],
    };
    core.set(change('R'), &mut backend);
    assert_eq!(
        core.status.read().operations[&'R'].outcome,
        DriveIndexingWriteOutcome::Confirmed { enabled: true }
    );
}

#[test]
fn confirmed_failure_leaves_the_previous_policy_and_finishes_the_operation() {
    let mut core = core();
    let error = DriveIndexingError::service("Drive configuration update failed".into());
    let mut backend = Fake {
        reads: VecDeque::new(),
        write: Err(error.clone()),
        writes: vec![],
    };
    assert!(!core.set(change('R'), &mut backend));
    assert!(core.policy.is_enabled('R'));
    assert_eq!(
        core.status.read().operations[&'R'].outcome,
        DriveIndexingWriteOutcome::Failed(error)
    );
}

#[test]
fn transport_and_busy_failures_never_mean_unsupported() {
    for error in [
        DriveIndexingError::transport("Search service timeout"),
        DriveIndexingError::service("Server busy, try again later".into()),
    ] {
        let mut core = core();
        core.refresh(&mut Fake {
            reads: VecDeque::from([Err(error)]),
            write: Ok(()),
            writes: vec![],
        });
        assert_eq!(
            core.policy.snapshot().availability,
            DriveIndexingAvailability::Offline
        );
        assert!(!core.policy.snapshot().legacy_service);
    }
}

#[test]
fn successful_periodic_refresh_observes_other_instances_without_any_write() {
    let mut core = core();
    let mut backend = Fake {
        reads: VecDeque::from([Ok(config(false))]),
        write: Ok(()),
        writes: vec![],
    };
    core.refresh(&mut backend);
    assert!(!core.policy.is_enabled('R'));
    assert!(backend.writes.is_empty());
}

#[test]
fn refresh_requests_are_bounded_and_coalesced_without_waiting() {
    let (requests, receiver) = bounded(32);
    let worker = DriveIndexingWorker {
        requests,
        status: Arc::new(RwLock::new(DriveIndexingWorkerStatus::default())),
        running: Arc::new(AtomicBool::new(true)),
        refresh_queued: Arc::new(AtomicBool::new(false)),
    };
    for _ in 0..100 {
        worker.refresh().unwrap();
    }
    assert_eq!(receiver.len(), 1);
    assert!(worker.set(change('é')).is_err());
    assert_eq!(receiver.len(), 1);
    worker.set(change('R')).unwrap();
    assert!(worker.set(change('R')).is_err());
    assert_eq!(receiver.len(), 2);
}

#[test]
fn a_periodic_read_cannot_confirm_a_queued_write_that_has_not_been_sent() {
    let mut core = core();
    core.status.write().operations.insert(
        'R',
        DriveIndexingOperation {
            request_id: 1,
            requested_enabled: false,
            outcome: DriveIndexingWriteOutcome::Queued,
        },
    );
    let mut backend = Fake {
        reads: VecDeque::from([Ok(config(true))]),
        write: Ok(()),
        writes: vec![],
    };
    core.refresh(&mut backend);
    assert_eq!(
        core.status.read().operations[&'R'].outcome,
        DriveIndexingWriteOutcome::Queued
    );
    assert!(backend.writes.is_empty());
}

#[test]
fn a_service_downgrade_finishes_unreconcilable_operations_without_rewriting_the_cache() {
    let mut core = core();
    let cache = core.policy.snapshot().cache_json();
    let mut backend = Fake {
        reads: VecDeque::from([Err(DriveIndexingError::new(
            DriveIndexingErrorKind::Unsupported,
            "legacy",
        ))]),
        write: Err(DriveIndexingError::transport("timeout")),
        writes: vec![],
    };
    core.set(change('R'), &mut backend);
    assert!(matches!(
        core.status.read().operations[&'R'].outcome,
        DriveIndexingWriteOutcome::Failed(_)
    ));
    assert_eq!(core.policy.snapshot().cache_json(), cache);
}
