use super::client_builder;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

fn server(reply: impl FnOnce(TcpStream) + Send + 'static) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
            assert!(request.len() < 16 * 1024);
        }
        reply(stream);
    });
    (url, handle)
}

fn client(timeout: Duration) -> reqwest::blocking::Client {
    // Only this loopback test relaxes HTTPS and bypasses machine proxy settings.
    client_builder()
        .https_only(false)
        .no_proxy()
        .timeout(timeout)
        .build()
        .unwrap()
}

#[test]
fn blocking_client_timeout_allows_a_continuous_body_longer_than_its_timeout() {
    let timeout = Duration::from_secs(1);
    let (url, server) = server(|mut stream| {
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\n")
            .unwrap();
        stream.write_all(b"a").unwrap();
        for _ in 0..5 {
            thread::sleep(Duration::from_millis(300));
            stream.write_all(b"a").unwrap();
        }
    });
    let started = std::time::Instant::now();
    let mut response = client(timeout).get(url).send().unwrap();
    let mut bytes = Vec::new();
    let result = response.read_to_end(&mut bytes);
    server.join().unwrap();
    result.unwrap();
    assert_eq!(bytes, b"aaaaaa");
    assert!(started.elapsed() > timeout);
}

#[test]
fn blocking_client_timeout_interrupts_a_stalled_body() {
    let (release, wait) = mpsc::channel();
    let (url, server) = server(move |mut stream| {
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\na")
            .unwrap();
        let _ = wait.recv_timeout(Duration::from_secs(15));
    });
    // Leave enough scheduling slack when the complete workspace runs in parallel.
    let mut response = client(Duration::from_secs(2)).get(url).send().unwrap();
    let mut bytes = Vec::new();
    let result = response.read_to_end(&mut bytes);
    let _ = release.send(());
    server.join().unwrap();
    assert!(result.is_err());
    assert_eq!(bytes, b"a");
}

#[test]
fn blocking_client_timeout_bounds_waiting_for_headers() {
    let (release, wait) = mpsc::channel();
    let (url, server) = server(move |_stream| {
        let _ = wait.recv_timeout(Duration::from_secs(15));
    });
    let result = client(Duration::from_secs(2)).get(url).send();
    let _ = release.send(());
    server.join().unwrap();
    assert!(result.unwrap_err().is_timeout());
}
