use std::io::Read;
use std::time::Duration;

use mtt_search_protocol::{encode_message, SearchResponse};

use super::super::super::connection_lifecycle::ConnectionOutcome;
use super::super::super::WatchdogEvent;
use super::super::{send_response, PIPE_BUFFER_SIZE};
use super::fixture::Connection;
use super::lifecycle_worker::LifecycleWorker;

#[test]
fn unread_response_is_canceled_with_the_deadline_still_active() {
    let connection = Connection::new();
    let task = LifecycleWorker::start(
        connection.clone_server(),
        Duration::from_millis(200),
        |pipe, _| send_response(pipe, &SearchResponse::Pong).unwrap(),
    );
    assert_eq!(task.wait(), ConnectionOutcome::TimedOut);
}

#[test]
fn non_reader_cannot_hold_a_write_larger_than_the_pipe_buffer_indefinitely() {
    let connection = Connection::new();
    let task = LifecycleWorker::start(
        connection.clone_server(),
        Duration::from_millis(200),
        |pipe, _| {
            let response = SearchResponse::Error("x".repeat(PIPE_BUFFER_SIZE as usize * 4));
            let _ = send_response(pipe, &response);
        },
    );
    assert_eq!(task.wait(), ConnectionOutcome::TimedOut);
}

#[test]
fn immediate_reader_receives_a_complete_reply_before_disconnect() {
    let mut connection = Connection::new();
    let expected = encode_message(&SearchResponse::Pong).unwrap();
    let task = LifecycleWorker::start(
        connection.clone_server(),
        Duration::from_secs(1),
        |pipe, _| send_response(pipe, &SearchResponse::Pong).unwrap(),
    );
    let mut received = vec![0; expected.len()];
    connection.client.read_exact(&mut received).unwrap();
    assert_eq!(received, expected);
    assert_eq!(task.wait(), ConnectionOutcome::Finished);
}

#[test]
fn delayed_partial_reader_keeps_its_reply_until_it_finishes() {
    let mut connection = Connection::new();
    let response = SearchResponse::Error("Authorization failed".into());
    let expected = encode_message(&response).unwrap();
    let task = LifecycleWorker::start(
        connection.clone_server(),
        Duration::from_secs(1),
        move |pipe, _| send_response(pipe, &response).unwrap(),
    );
    task.assert_running(Duration::from_millis(50));
    let mut received = vec![0; expected.len()];
    connection.client.read_exact(&mut received[..4]).unwrap();
    task.assert_running(Duration::from_millis(50));
    connection.client.read_exact(&mut received[4..]).unwrap();
    assert_eq!(received, expected);
    assert_eq!(task.wait(), ConnectionOutcome::Finished);
}

#[test]
fn response_larger_than_the_pipe_buffer_is_delivered_without_truncation() {
    let mut connection = Connection::new();
    let response = SearchResponse::Error("x".repeat(PIPE_BUFFER_SIZE as usize * 4));
    let expected = encode_message(&response).unwrap();
    let task = LifecycleWorker::start(
        connection.clone_server(),
        Duration::from_secs(2),
        move |pipe, _| send_response(pipe, &response).unwrap(),
    );
    let mut received = vec![0; expected.len()];
    connection.client.read_exact(&mut received).unwrap();
    assert_eq!(received, expected);
    assert_eq!(task.wait(), ConnectionOutcome::Finished);
}

#[test]
fn operation_extension_applies_to_buffered_reply_delivery_too() {
    let mut connection = Connection::new();
    let expected = encode_message(&SearchResponse::Pong).unwrap();
    let task = LifecycleWorker::start(
        connection.clone_server(),
        Duration::from_millis(200),
        |pipe, events| {
            events
                .send(WatchdogEvent::Extend(Duration::from_secs(1)))
                .unwrap();
            send_response(pipe, &SearchResponse::Pong).unwrap();
        },
    );
    task.assert_running(Duration::from_millis(300));
    let mut received = vec![0; expected.len()];
    connection.client.read_exact(&mut received).unwrap();
    assert_eq!(received, expected);
    assert_eq!(task.wait(), ConnectionOutcome::Finished);
}

#[test]
fn handler_panic_finishes_without_leaking_the_deadline_worker() {
    let connection = Connection::new();
    let task = LifecycleWorker::start(connection.clone_server(), Duration::from_secs(1), |_, _| {
        panic!("synthetic handler failure")
    });
    assert_eq!(task.wait(), ConnectionOutcome::HandlerPanicked);
}

#[test]
fn healthy_connection_completes_after_another_connection_times_out() {
    let non_reader = Connection::new();
    let expired = LifecycleWorker::start(
        non_reader.clone_server(),
        Duration::from_millis(100),
        |pipe, _| send_response(pipe, &SearchResponse::Pong).unwrap(),
    );
    assert_eq!(expired.wait(), ConnectionOutcome::TimedOut);

    let mut healthy = Connection::new();
    let task = LifecycleWorker::start(healthy.clone_server(), Duration::from_secs(1), |pipe, _| {
        send_response(pipe, &SearchResponse::Pong).unwrap()
    });
    let expected = encode_message(&SearchResponse::Pong).unwrap();
    let mut received = vec![0; expected.len()];
    healthy.client.read_exact(&mut received).unwrap();
    assert_eq!(received, expected);
    assert_eq!(task.wait(), ConnectionOutcome::Finished);
}

#[test]
fn disconnected_client_does_not_leave_a_deadline_worker_running() {
    let connection = Connection::new();
    let (disconnected, receiver) = std::sync::mpsc::channel();
    let task = LifecycleWorker::start(
        connection.clone_server(),
        Duration::from_secs(1),
        move |pipe, _| {
            receiver.recv_timeout(Duration::from_secs(1)).unwrap();
            assert!(send_response(pipe, &SearchResponse::Pong).is_err());
        },
    );
    drop(connection);
    disconnected.send(()).unwrap();
    assert_eq!(task.wait(), ConnectionOutcome::Finished);
}
