use std::io::Read;
use std::time::{Duration, Instant};

use mtt_search_protocol::{encode_message, SearchResponse};

use super::super::super::connection_lifecycle::ConnectionOutcome;
use super::super::super::connection_rejection::{RejectionPool, RejectionStatus};
use super::super::send_response;
use super::fixture::Connection;
use super::lifecycle_worker::LifecycleWorker;

fn wait_until_idle(pool: &RejectionPool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while pool.active_count() != 0 {
        assert!(Instant::now() < deadline, "rejection slot was not released");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn rejection_delivers_its_original_error_to_a_reading_client() {
    let mut connection = Connection::new();
    let pool = RejectionPool::new(1, Duration::from_secs(1));
    let response = SearchResponse::Error("Server busy, try again later".into());
    let expected = encode_message(&response).unwrap();
    assert_eq!(
        pool.reject(connection.clone_server(), response).unwrap(),
        RejectionStatus::Scheduled
    );
    let mut received = vec![0; expected.len()];
    connection.client.read_exact(&mut received).unwrap();
    assert_eq!(received, expected);
    wait_until_idle(&pool);
}

#[test]
fn unread_rejection_does_not_block_a_healthy_connection() {
    let non_reader = Connection::new();
    let pool = RejectionPool::new(1, Duration::from_millis(500));
    assert_eq!(
        pool.reject(
            non_reader.clone_server(),
            SearchResponse::Error("Server busy, try again later".into()),
        )
        .unwrap(),
        RejectionStatus::Scheduled
    );
    let mut healthy = Connection::new();
    let task = LifecycleWorker::start(healthy.clone_server(), Duration::from_secs(1), |pipe, _| {
        send_response(pipe, &SearchResponse::Pong).unwrap()
    });
    let expected = encode_message(&SearchResponse::Pong).unwrap();
    let mut received = vec![0; expected.len()];
    healthy.client.read_exact(&mut received).unwrap();
    assert_eq!(received, expected);
    assert_eq!(task.wait(), ConnectionOutcome::Finished);
    wait_until_idle(&pool);
}

#[test]
fn saturated_rejection_capacity_drops_excess_without_another_worker() {
    let first = Connection::new();
    let mut excess = Connection::new();
    let pool = RejectionPool::new(1, Duration::from_secs(2));
    assert_eq!(
        pool.reject(first.clone_server(), SearchResponse::Error("busy".into()))
            .unwrap(),
        RejectionStatus::Scheduled
    );
    assert_eq!(pool.active_count(), 1);
    assert_eq!(
        pool.reject(excess.clone_server(), SearchResponse::Error("busy".into()))
            .unwrap(),
        RejectionStatus::DroppedAtCapacity
    );
    let mut byte = [0];
    assert!(excess.client.read_exact(&mut byte).is_err());
    assert_eq!(pool.active_count(), 1);
    wait_until_idle(&pool);
}

#[test]
fn rejection_slot_is_reusable_after_an_unread_reply_times_out() {
    let non_reader = Connection::new();
    let pool = RejectionPool::new(1, Duration::from_millis(200));
    assert_eq!(
        pool.reject(non_reader.clone_server(), SearchResponse::Pong)
            .unwrap(),
        RejectionStatus::Scheduled
    );
    wait_until_idle(&pool);

    let mut healthy = Connection::new();
    assert_eq!(
        pool.reject(healthy.clone_server(), SearchResponse::Pong)
            .unwrap(),
        RejectionStatus::Scheduled
    );
    let expected = encode_message(&SearchResponse::Pong).unwrap();
    let mut received = vec![0; expected.len()];
    healthy.client.read_exact(&mut received).unwrap();
    assert_eq!(received, expected);
    wait_until_idle(&pool);
}
