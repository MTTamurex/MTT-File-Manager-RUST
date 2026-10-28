use super::*;
use crate::tabs::TabState;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn nonblocking_listener_waits_for_delayed_child_authentication() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let (accepted_tx, accepted_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    accepted_tx.send(()).unwrap();
                    return authenticate_stream(&mut stream, TOKEN.as_bytes(), deadline);
                }
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => return Err(error),
            }
        }
    });
    let mut stream = TcpStream::connect(address).unwrap();
    accepted_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    // A new process is not guaranteed to send its token before accept returns.
    std::thread::sleep(Duration::from_millis(50));
    let _ = stream.write_all(TOKEN.as_bytes());
    assert!(server.join().unwrap().unwrap());
}

fn exchange(ack: &[u8; 5]) -> io::Result<()> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let mut stream = TcpStream::connect(address).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        assert!(authenticate_stream(
            &mut stream,
            TOKEN.as_bytes(),
            Instant::now() + Duration::from_secs(2)
        )
        .unwrap());
        let transfer = TabTransfer::from_tab(&TabState::new_at_computer(0));
        send_payload(
            &mut stream,
            &serde_json::to_vec(&transfer).unwrap(),
            Duration::from_secs(2),
        )
    });
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let received = read_transfer(&mut stream, TOKEN).unwrap();
    assert_eq!(received.session.tabs.len(), 1);
    // Window initialization happens after receipt, not before send_payload
    // starts waiting. Exercise that gap on the actual accepted socket mode.
    std::thread::sleep(Duration::from_millis(50));
    stream.write_all(ack).unwrap();
    server.join().unwrap()
}

#[test]
fn source_waits_for_the_ready_acknowledgement() {
    assert!(exchange(b"READY").is_ok());
    assert!(exchange(b"ERROR").is_err());
}

#[test]
fn authenticated_transport_also_preserves_complete_multi_tab_sessions() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let mut manager = crate::tabs::TabManager::new_at_path(r"C:\Example");
    manager.new_tab_at(r"D:\Other");
    manager.tabs[0].pinned = true;
    let expected = TabManagerSession::from_tab_manager(&manager);
    let bytes = expected.to_json().unwrap().into_bytes();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        authenticate_stream(
            &mut stream,
            TOKEN.as_bytes(),
            Instant::now() + Duration::from_secs(2),
        )
        .unwrap();
        send_payload(&mut stream, &bytes, Duration::from_secs(2)).unwrap();
    });
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let bytes = read_payload(&mut stream, TOKEN).unwrap();
    assert_eq!(
        TabManagerSession::from_json(std::str::from_utf8(&bytes).unwrap()).unwrap(),
        expected
    );
    stream.write_all(b"READY").unwrap();
    server.join().unwrap();
}

#[test]
fn invalid_tokens_are_rejected_before_connecting() {
    assert!(receive_startup(0, "").is_err());
    assert!(receive_startup(0, "not-a-transfer-token").is_err());
}

#[test]
fn oversized_payload_is_rejected_before_allocation() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut token = [0; 32];
        stream.read_exact(&mut token).unwrap();
        stream
            .write_all(&((MAX_BYTES + 1) as u32).to_le_bytes())
            .unwrap();
    });
    let mut stream = TcpStream::connect(address).unwrap();
    assert!(read_transfer(&mut stream, TOKEN).is_err());
    server.join().unwrap();
}

#[test]
fn missing_acknowledgement_times_out_without_reporting_success() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        send_payload(&mut stream, b"{}", Duration::from_millis(50))
    });
    let mut stream = TcpStream::connect(address).unwrap();
    let mut payload = [0; 6];
    stream.read_exact(&mut payload).unwrap();
    assert!(server.join().unwrap().is_err());
}

#[test]
fn expired_deadline_cannot_start_another_blocking_operation() {
    assert!(remaining(Instant::now() - Duration::from_millis(1)).is_err());
    assert!(remaining(Instant::now() + Duration::from_secs(1)).is_ok());
}

#[test]
fn transfer_child_process() {
    let Ok(port) = std::env::var("MTT_TEST_TAB_TRANSFER_PORT") else {
        return;
    };
    let token = std::env::var("MTT_TEST_TAB_TRANSFER_TOKEN").unwrap();
    receive_startup(port.parse().unwrap(), &token).unwrap();
    let startup = STARTUP.get().unwrap();
    assert_eq!(startup.transfer.session.tabs.len(), 1);
    std::thread::sleep(Duration::from_millis(50));
    startup
        .stream
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .write_all(b"READY")
        .unwrap();
}

#[test]
fn nonblocking_listener_transfers_to_a_separate_process() {
    use std::process::Stdio;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "app::tab_transfer::tests::transfer_child_process",
        ])
        .env(
            "MTT_TEST_TAB_TRANSFER_PORT",
            listener.local_addr().unwrap().port().to_string(),
        )
        .env("MTT_TEST_TAB_TRANSFER_TOKEN", TOKEN)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let transfer = TabTransfer::from_tab(&TabState::new_at_computer(0));
    let result = send_session(
        &listener,
        &mut child,
        TOKEN.as_bytes(),
        &serde_json::to_vec(&transfer).unwrap(),
    );
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().unwrap();
    assert!(result.is_ok(), "transfer to child failed: {result:?}");
    assert!(status.success());
}
