//! Test the production service lifecycle with contained, disposable IPC peers.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

static SINGLE_SPAWNS: AtomicUsize = AtomicUsize::new(0);
static QUEUED_SPAWNS: AtomicUsize = AtomicUsize::new(0);

fn single_peer() -> Command {
    SINGLE_SPAWNS.fetch_add(1, Ordering::Relaxed);
    echo_peer()
}

fn queued_peer() -> Command {
    QUEUED_SPAWNS.fetch_add(1, Ordering::Relaxed);
    echo_peer()
}

fn service(command: fn() -> Command) -> Service {
    let mut client = extractor();
    client.set_test_command(command);
    Service::new(client)
}

#[test]
fn service_starts_on_demand_and_recreates_after_each_drained_queue() {
    SINGLE_SPAWNS.store(0, Ordering::Relaxed);
    let service = service(single_peer);
    assert!(service.client.lock().process_id().is_none());
    assert_eq!(SINGLE_SPAWNS.load(Ordering::Relaxed), 0);
    for expected in 1..=2 {
        let frame = service
            .extract_until(fixture(), 256, &|| false, Instant::now() + REQUEST_TIMEOUT)
            .unwrap();
        assert_eq!(frame.rgba, [1, 2, 3, 255]);
        assert_eq!(SINGLE_SPAWNS.load(Ordering::Relaxed), expected);
        assert_eq!(service.pending.load(Ordering::Acquire), 0);
        assert!(service.client.lock().process_id().is_none());
        assert!(!service.control.stopped());
    }
}

#[test]
fn last_pending_request_reaps_the_child_and_allows_future_requests() {
    let service = service(echo_peer);
    let batch = service.register();
    service
        .extract_until(fixture(), 256, &|| false, Instant::now() + REQUEST_TIMEOUT)
        .unwrap();
    let first_id = service.client.lock().process_id().unwrap();
    let first = process_handle(&service.client.lock());
    drop(batch);
    assert!(service.client.lock().process_id().is_none());
    assert_exited(&first);

    let next_batch = service.register();
    service
        .extract_until(fixture(), 256, &|| false, Instant::now() + REQUEST_TIMEOUT)
        .unwrap();
    let second = process_handle(&service.client.lock());
    assert_ne!(service.client.lock().process_id().unwrap(), first_id);
    drop(next_batch);
    assert_exited(&second);
    assert!(service.client.lock().process_id().is_none());
}

#[test]
fn queued_requests_share_one_child_then_release_it_without_an_idle_delay() {
    QUEUED_SPAWNS.store(0, Ordering::Relaxed);
    let service = service(queued_peer);
    let owner = service.client.lock();
    let (done, receiver) = mpsc::channel();
    std::thread::scope(|scope| {
        for _ in 0..3 {
            let done = done.clone();
            let service = &service;
            scope.spawn(move || {
                done.send(service.extract_until(
                    fixture(),
                    256,
                    &|| false,
                    Instant::now() + REQUEST_TIMEOUT,
                ))
                .unwrap();
            });
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while service.pending.load(Ordering::Acquire) != 3 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        let queued = service.pending.load(Ordering::Acquire);
        drop(owner);
        for _ in 0..3 {
            assert_eq!(
                receiver
                    .recv_timeout(REQUEST_TIMEOUT)
                    .unwrap()
                    .unwrap()
                    .rgba,
                [1, 2, 3, 255]
            );
        }
        assert_eq!(queued, 3);
    });
    assert_eq!(QUEUED_SPAWNS.load(Ordering::Relaxed), 1);
    assert_eq!(service.pending.load(Ordering::Acquire), 0);
    assert!(service.client.lock().process_id().is_none());
}

#[test]
fn cancelled_or_expired_requests_never_start_a_child() {
    fn unexpected() -> Command {
        panic!("Cancelled request tried to spawn a peer");
    }
    let service = service(unexpected);
    assert_eq!(
        service
            .extract_until(fixture(), 256, &|| true, Instant::now() + REQUEST_TIMEOUT)
            .unwrap_err(),
        Error::Cancelled
    );
    assert_eq!(
        service
            .extract_until(
                fixture(),
                256,
                &|| false,
                Instant::now() - Duration::from_millis(1)
            )
            .unwrap_err(),
        Error::QueueTimeout
    );
    assert_eq!(service.pending.load(Ordering::Acquire), 0);
    assert!(service.client.lock().process_id().is_none());
}

#[test]
fn failed_request_leaves_no_resident_child_and_the_next_one_can_succeed() {
    fn crashed() -> Command {
        peer("exit 7")
    }
    let service = service(crashed);
    assert_eq!(
        service
            .extract_until(fixture(), 256, &|| false, Instant::now() + REQUEST_TIMEOUT)
            .unwrap_err(),
        Error::Transport
    );
    assert_eq!(service.pending.load(Ordering::Acquire), 0);
    assert!(service.client.lock().process_id().is_none());
    service.client.lock().set_test_command(echo_peer);
    assert!(service
        .extract_until(fixture(), 256, &|| false, Instant::now() + REQUEST_TIMEOUT)
        .is_ok());
    assert!(service.client.lock().process_id().is_none());
}
