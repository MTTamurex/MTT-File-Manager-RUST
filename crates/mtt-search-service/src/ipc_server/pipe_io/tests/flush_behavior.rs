//! Retain Windows flush behavior and the historical cleanup hazard as controls.
//! Only uniquely named local test pipes are used; no service/probe is launched.

use std::io::Read;
use std::sync::mpsc;
use std::time::Duration;

use mtt_search_protocol::{decode_message, encode_message, SearchResponse};

use super::super::super::{watchdog_timed_out, WatchdogEvent};
use super::super::send_response;
use super::fixture::Connection;
use super::flush_worker::FlushWorker;

const OBSERVATION: Duration = Duration::from_millis(100);

#[test]
fn unread_reply_remains_pending_after_production_watchdog_is_marked_done() {
    let connection = Connection::new();
    send_response(connection.server(), &SearchResponse::Pong).unwrap();
    let (events, receiver) = mpsc::channel();
    // Reproduce the previous normal-handler cleanup ordering: watchdog is finished
    // and joined before flushing, so it cannot time out that flush.
    events.send(WatchdogEvent::Done).unwrap();
    let watchdog =
        std::thread::spawn(move || watchdog_timed_out(&receiver, Duration::from_millis(25)));
    assert!(!watchdog.join().unwrap());

    let flush = FlushWorker::start(connection.clone_server());
    flush.assert_pending(OBSERVATION);
    flush.cancel();
    // Forced cancellation/disconnection only establishes bounded test cleanup,
    // not successful delivery of an unread response.
    let _ = flush.wait();
}

#[test]
fn rejection_response_also_waits_for_a_non_reading_client() {
    let connection = Connection::new();
    send_response(
        connection.server(),
        &SearchResponse::Error("Server busy, try again later".into()),
    )
    .unwrap();
    // Reproduce the previous pre-watchdog rejection flushing. Production now
    // uses a bounded rejection worker; this control intentionally calls raw flush.
    let flush = FlushWorker::start(connection.clone_server());
    flush.assert_pending(OBSERVATION);
    flush.cancel();
    let _ = flush.wait();
}

#[test]
fn delayed_partial_reader_receives_the_complete_response_before_flush_finishes() {
    let mut connection = Connection::new();
    let response = SearchResponse::Error("Authorization failed".into());
    let expected = encode_message(&response).unwrap();
    send_response(connection.server(), &response).unwrap();
    let flush = FlushWorker::start(connection.clone_server());
    flush.assert_pending(OBSERVATION);

    let mut prefix = [0u8; 4];
    connection.client.read_exact(&mut prefix).unwrap();
    assert_eq!(u32::from_le_bytes(prefix) as usize, expected.len() - 4);
    // Reading only the length does not consume the whole buffered reply.
    flush.assert_pending(OBSERVATION);
    let mut payload = vec![0; expected.len() - 4];
    connection.client.read_exact(&mut payload).unwrap();
    assert_eq!(payload, expected[4..]);
    assert!(matches!(
        decode_message(&payload),
        Ok(SearchResponse::Error(message)) if message == "Authorization failed"
    ));
    assert_eq!(flush.wait(), Ok(()));
}

#[test]
fn immediate_reader_receives_the_complete_response_and_flush_finishes() {
    let mut connection = Connection::new();
    let expected = encode_message(&SearchResponse::Pong).unwrap();
    send_response(connection.server(), &SearchResponse::Pong).unwrap();
    let flush = FlushWorker::start(connection.clone_server());
    let mut received = vec![0; expected.len()];
    connection.client.read_exact(&mut received).unwrap();
    assert_eq!(received, expected);
    assert_eq!(flush.wait(), Ok(()));
}
