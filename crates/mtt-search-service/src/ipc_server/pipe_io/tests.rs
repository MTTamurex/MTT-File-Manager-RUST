//! Real local pipes with unique names; never connects to the installed service.
//! Uses the current test process as both peers, without spawning executables.

mod candidate_acl;
mod fixture;
mod restricted_token;

use std::io::{Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    FILE_CREATE_PIPE_INSTANCE, FILE_GENERIC_READ, FILE_WRITE_DATA,
};
use windows::Win32::System::Pipes::{
    GetNamedPipeClientProcessId, GetNamedPipeServerProcessId, PeekNamedPipe,
};

use fixture::{unique_name, Connection};
use mtt_search_protocol::{
    decode_message, encode_message, SearchRequest, SearchResponse, PIPE_CLIENT_ACCESS,
};

use super::{create_pipe_named, read_message, send_response, MAX_REQUEST_PAYLOAD};

#[test]
fn client_requests_data_access_without_the_create_instance_bit() {
    assert_eq!(PIPE_CLIENT_ACCESS, FILE_GENERIC_READ.0 | FILE_WRITE_DATA.0);
    assert_eq!(PIPE_CLIENT_ACCESS & FILE_CREATE_PIPE_INSTANCE.0, 0);
    assert_eq!(PIPE_CLIENT_ACCESS & 0xC000_0000, 0); // No generic read/write mapping.
}

#[test]
fn explicit_client_rights_allow_response_polling() {
    let connection = Connection::new();
    send_response(connection.server(), &SearchResponse::Pong).unwrap();
    let mut available = 0;
    unsafe {
        PeekNamedPipe(
            HANDLE(connection.client.as_raw_handle()),
            None,
            0,
            None,
            Some(&mut available),
            None,
        )
        .unwrap();
    }
    assert_eq!(
        available as usize,
        encode_message(&SearchResponse::Pong).unwrap().len()
    );
}

#[test]
fn ping_and_response_round_trip_uses_production_transport() {
    let mut connection = Connection::new();
    connection
        .client
        .write_all(&encode_message(&SearchRequest::Ping).unwrap())
        .unwrap();
    let payload = read_message(connection.server()).expect("read Ping frame");
    assert!(matches!(decode_message(&payload), Ok(SearchRequest::Ping)));

    send_response(connection.server(), &SearchResponse::Pong).unwrap();
    let mut length = [0u8; 4];
    connection.client.read_exact(&mut length).unwrap();
    let size = u32::from_le_bytes(length) as usize;
    assert!(size > 0 && size <= MAX_REQUEST_PAYLOAD);
    let mut payload = vec![0; size];
    connection.client.read_exact(&mut payload).unwrap();
    assert!(matches!(decode_message(&payload), Ok(SearchResponse::Pong)));
}

#[test]
fn invalid_frame_sizes_are_rejected_without_reading_a_body() {
    for size in [0, MAX_REQUEST_PAYLOAD as u32 + 1, u32::MAX] {
        let mut connection = Connection::new();
        connection.client.write_all(&size.to_le_bytes()).unwrap();
        assert!(read_message(connection.server()).is_none());
    }
}

#[test]
fn maximum_sized_body_is_accepted() {
    let mut connection = Connection::new();
    let expected = vec![0x5a; MAX_REQUEST_PAYLOAD];
    let handle = connection.server();
    // Reading concurrently avoids filling the pipe buffer while writing the
    // maximum payload plus its length header. The fixture bounds both ends.
    std::thread::scope(|scope| {
        let client = &mut connection.client;
        let body = &expected;
        let writer = scope.spawn(move || {
            client
                .write_all(&(body.len() as u32).to_le_bytes())
                .unwrap();
            client.write_all(body).unwrap();
        });
        assert_eq!(read_message(handle).unwrap(), expected);
        writer.join().unwrap();
    });
}

#[test]
fn invalid_request_encoding_is_rejected_after_transport_read() {
    let mut connection = Connection::new();
    let payload = u32::MAX.to_le_bytes();
    connection.client.write_all(&4u32.to_le_bytes()).unwrap();
    connection.client.write_all(&payload).unwrap();
    let received = read_message(connection.server()).unwrap();
    assert!(decode_message::<SearchRequest>(&received).is_err());
}

#[test]
fn client_and_server_pids_come_from_the_connected_pipe() {
    let connection = Connection::new();
    let mut client_pid = 0;
    let mut server_pid = 0;
    unsafe {
        GetNamedPipeClientProcessId(connection.server(), &mut client_pid).unwrap();
        GetNamedPipeServerProcessId(HANDLE(connection.client.as_raw_handle()), &mut server_pid)
            .unwrap();
    }
    assert_eq!(client_pid, std::process::id());
    assert_eq!(server_pid, std::process::id());
}

#[test]
fn test_executable_is_not_a_trusted_file_manager_client() {
    let mut connection = Connection::new();
    connection
        .client
        .write_all(&encode_message(&SearchRequest::Ping).unwrap())
        .unwrap();
    assert!(read_message(connection.server()).is_some());
    let error = crate::ipc_authorization::trusted_file_manager_client(connection.server())
        .expect_err("the test executable must not qualify as the installed app");
    assert!(error.starts_with("Client image basename is '"));
}

#[test]
fn first_instance_flag_rejects_an_existing_test_pipe() {
    let name = unique_name();
    assert_ne!(name, mtt_search_protocol::PIPE_NAME);
    let handle = create_pipe_named(&name, true).unwrap();
    let _server = unsafe { OwnedHandle::from_raw_handle(handle.0) };
    assert!(create_pipe_named(&name, true).is_err());
}

#[test]
fn server_disconnect_rejects_further_reads_and_writes() {
    let connection = Connection::new();
    unsafe {
        windows::Win32::System::Pipes::DisconnectNamedPipe(connection.server()).unwrap();
    }
    assert!(read_message(connection.server()).is_none());
    assert!(send_response(connection.server(), &SearchResponse::Pong).is_err());
}
