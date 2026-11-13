use super::*;
use crate::infrastructure::copy_engine::{CopyOperation, CopyRuntime};
use std::sync::Arc;
use std::time::Instant;

pub(crate) fn wait_for_request(operation: &CopyOperation) -> Request {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(request) = operation.deletion.as_ref().unwrap().confirmation.pending() {
            return request;
        }
        assert!(Instant::now() < deadline, "Confirmation was not published");
        std::thread::yield_now();
    }
}

#[test]
fn no_work_is_authorized_without_an_explicit_current_response() {
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete(vec![PathBuf::from("source/file.txt")], true);
    let operation = job.operation.clone();
    let worker = std::thread::spawn(move || ask(&operation, Kind::Permanent, &operation.sources));
    let request = wait_for_request(&job.operation);
    let confirmation = &job.operation.deletion.as_ref().unwrap().confirmation;
    assert_eq!(request.kind, Kind::Permanent);
    assert!(!confirmation.respond(request.id.wrapping_add(1), true));
    assert!(!worker.is_finished());
    assert!(confirmation.respond(request.id, false));
    assert!(!worker.join().unwrap());
    assert!(!confirmation.respond(request.id, true));
}

#[test]
fn cancellation_unblocks_worker_without_a_response_and_clears_prompt() {
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete(vec![PathBuf::from("source/file.txt")], true);
    let operation = job.operation.clone();
    let worker = std::thread::spawn(move || ask(&operation, Kind::Permanent, &operation.sources));
    let request = wait_for_request(&job.operation);
    job.operation.progress.cancel();
    assert!(!worker.join().unwrap());
    let confirmation = &job.operation.deletion.as_ref().unwrap().confirmation;
    assert!(confirmation.pending().is_none());
    assert!(!confirmation.respond(request.id, true));
}

#[test]
fn recycle_consent_does_not_authorize_a_later_permanent_fallback() {
    let runtime = Arc::new(CopyRuntime::default());
    let job = runtime.start_delete(vec![PathBuf::from("source/file.txt")], false);
    let operation = job.operation.clone();
    let worker = std::thread::spawn(move || {
        assert!(ask(&operation, Kind::Recycle, &operation.sources));
        ask(&operation, Kind::RecycleUnavailable, &operation.sources)
    });
    let first = wait_for_request(&job.operation);
    let confirmation = &job.operation.deletion.as_ref().unwrap().confirmation;
    assert!(confirmation.respond(first.id, true));
    let second = wait_for_request(&job.operation);
    assert_ne!(first.id, second.id);
    assert_eq!(second.kind, Kind::RecycleUnavailable);
    assert!(!confirmation.respond(first.id, true));
    assert!(confirmation.respond(second.id, false));
    assert!(!worker.join().unwrap());
}
