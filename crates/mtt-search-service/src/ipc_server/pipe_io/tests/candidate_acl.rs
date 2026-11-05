//! Regression tests for the adopted ACL, exercised only on unique test pipes.
//! The elevated test name is retained for the documented Sandbox command.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use mtt_search_protocol::{
    decode_message, encode_message, SearchRequest, SearchResponse, PIPE_CLIENT_ACCESS,
};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{GetLastError, LocalFree, ERROR_ACCESS_DENIED, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::{
    GetEffectiveRightsFromAclW, GetSecurityInfo, EXPLICIT_ACCESS_W, SE_KERNEL_OBJECT,
};
use windows::Win32::Security::{
    EqualSid, GetAce, ACCESS_ALLOWED_ACE, ACL, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    PSID,
};
use windows::Win32::Storage::FileSystem::{FILE_CREATE_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES};
use windows::Win32::System::Pipes::{
    CreateNamedPipeW, GetNamedPipeServerProcessId, PeekNamedPipe, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_WAIT,
};

use super::super::{
    create_pipe_named, read_message, send_response, with_pipe_access_entries, AclGuard, SidGuard,
    PIPE_BUFFER_SIZE, PIPE_MAX_INSTANCES, PIPE_OPEN_MODE,
};
use super::fixture::{unique_name, Connection};
use super::restricted_token::{assert_no_thread_token, without_admin_rights};

const SERVER_ACCESS: u32 = 0x001F01FF;

fn with_production_entries<T>(operation: impl FnOnce(&[EXPLICIT_ACCESS_W]) -> T) -> T {
    with_pipe_access_entries(|entries| Ok(operation(entries))).unwrap()
}

fn endpoint() -> (String, OwnedHandle) {
    let name = unique_name();
    assert_ne!(name, mtt_search_protocol::PIPE_NAME);
    let raw = create_pipe_named(&name, true).expect("create isolated production-policy endpoint");
    (name, unsafe { OwnedHandle::from_raw_handle(raw.0) })
}

fn open_client(name: &str, access: u32) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .access_mode(access)
        .custom_flags(0x00100000 | 0x00010000)
        .open(name)
}

#[test]
fn ordinary_client_can_exchange_data_poll_and_query_server_identity() {
    let (name, server) = endpoint();
    let client = without_admin_rights(|| open_client(&name, PIPE_CLIENT_ACCESS)).unwrap();
    let mut connection = Connection::from_parts(server, client);
    connection
        .client
        .write_all(&encode_message(&SearchRequest::Ping).unwrap())
        .unwrap();
    let payload = read_message(connection.server()).unwrap();
    assert!(matches!(decode_message(&payload), Ok(SearchRequest::Ping)));
    send_response(connection.server(), &SearchResponse::Pong).unwrap();
    let mut available = 0;
    let mut pid = 0;
    without_admin_rights(|| unsafe {
        PeekNamedPipe(
            HANDLE(connection.client.as_raw_handle()),
            None,
            0,
            None,
            Some(&mut available),
            None,
        )
        .unwrap();
        GetNamedPipeServerProcessId(HANDLE(connection.client.as_raw_handle()), &mut pid).unwrap();
    });
    let expected = encode_message(&SearchResponse::Pong).unwrap();
    assert_eq!(available as usize, expected.len());
    assert_eq!(pid, std::process::id());
    let mut received = vec![0; expected.len()];
    connection.client.read_exact(&mut received).unwrap();
    assert_eq!(received, expected);
}

#[test]
fn extra_client_access_request_does_not_allow_server_instance_creation() {
    let (name, _server) = endpoint();
    let result = without_admin_rights(|| {
        open_client(&name, PIPE_CLIENT_ACCESS | FILE_CREATE_PIPE_INSTANCE.0)
    });
    // Windows may accept the append bit on a client handle. Do not mistake
    // that outcome for permission to create another server-side instance.
    let _client = match result {
        Ok(client) => Some(client),
        Err(error) => {
            assert_eq!(error.raw_os_error(), Some(ERROR_ACCESS_DENIED.0 as i32));
            None
        }
    };
    assert_additional_instance_denied(&name);
}

#[test]
fn ordinary_client_cannot_create_an_additional_server_instance() {
    let (name, _server) = endpoint();
    assert_additional_instance_denied(&name);
}

fn assert_additional_instance_denied(name: &str) {
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let (denied, error) = without_admin_rights(|| unsafe {
        // Match the original instance's mode, limits and timeout exactly.
        // Mismatched pipe properties can also produce ERROR_ACCESS_DENIED,
        // which must not be mistaken for an authorization rejection.
        let raw = CreateNamedPipeW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(PIPE_OPEN_MODE),
            PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_MAX_INSTANCES,
            PIPE_BUFFER_SIZE,
            PIPE_BUFFER_SIZE,
            0,
            None,
        );
        let error = GetLastError();
        let denied = raw.is_invalid();
        if !denied {
            let _unexpected = OwnedHandle::from_raw_handle(raw.0);
        }
        (denied, error)
    });
    assert!(
        denied,
        "ordinary user unexpectedly created another instance"
    );
    assert_eq!(error, ERROR_ACCESS_DENIED);
}

#[test]
#[ignore = "requires an elevated Windows Sandbox test process; do not elevate the host tests"]
fn elevated_server_can_create_additional_instance() {
    let admins = SidGuard::allocate(2, 32, 544).unwrap();
    let mut member = windows::core::BOOL::default();
    unsafe { windows::Win32::Security::CheckTokenMembership(None, admins.0, &mut member) }.unwrap();
    assert!(
        member.as_bool(),
        "run this exact test elevated inside Sandbox only"
    );
    let (name, _first) = endpoint();
    let raw = create_pipe_named(&name, false)
        .expect("elevated server must retain rights to create its next instance");
    let _second = unsafe { OwnedHandle::from_raw_handle(raw.0) };
}

#[test]
fn legacy_generic_write_client_is_denied_for_ordinary_token() {
    let (name, _server) = endpoint();
    let error = without_admin_rights(|| open_client(&name, 0x80000000 | 0x40000000))
        .expect_err("legacy GENERIC_WRITE must not be silently allowed");
    assert_eq!(error.raw_os_error(), Some(ERROR_ACCESS_DENIED.0 as i32));
}

#[test]
fn production_entries_reserve_server_rights_for_system_and_administrators() {
    with_production_entries(|entries| {
        let acl = AclGuard::from_entries(entries).unwrap();
        for (index, expected) in [
            (0, PIPE_CLIENT_ACCESS),
            (1, SERVER_ACCESS),
            (2, SERVER_ACCESS),
        ] {
            let mut actual = 0;
            let status = unsafe {
                GetEffectiveRightsFromAclW(acl.as_ptr(), &entries[index].Trustee, &mut actual)
            };
            assert_eq!(status.0, 0);
            assert_eq!(actual, expected);
        }
    });
}

#[test]
fn created_pipe_has_exactly_the_expected_trustees_and_access_masks() {
    let (_name, server) = endpoint();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let mut dacl = std::ptr::null_mut::<ACL>();
    let status = unsafe {
        GetSecurityInfo(
            HANDLE(server.as_raw_handle()),
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut dacl),
            None,
            Some(&mut descriptor),
        )
    };
    assert_eq!(status.0, 0, "read security from the created pipe");
    struct DescriptorGuard(PSECURITY_DESCRIPTOR);
    impl Drop for DescriptorGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.0 .0)));
            }
        }
    }
    let _descriptor = DescriptorGuard(descriptor);
    assert!(
        !dacl.is_null(),
        "a null DACL would allow unrestricted access"
    );
    assert_eq!(unsafe { (*dacl).AceCount }, 3);

    // Independent role expectations: do not reuse the production role constructors.
    let users = SidGuard::allocate(1, 11, 0).unwrap(); // S-1-5-11
    let system = SidGuard::allocate(1, 18, 0).unwrap(); // S-1-5-18
    let admins = SidGuard::allocate(2, 32, 544).unwrap(); // S-1-5-32-544
    let expected = [
        (users.0, PIPE_CLIENT_ACCESS),
        (system.0, SERVER_ACCESS),
        (admins.0, SERVER_ACCESS),
    ];
    let mut seen = [false; 3];
    for index in 0..3 {
        let mut raw_ace = std::ptr::null_mut();
        unsafe { GetAce(dacl, index, &mut raw_ace) }.unwrap();
        assert!(!raw_ace.is_null());
        let ace = raw_ace.cast::<ACCESS_ALLOWED_ACE>();
        unsafe {
            assert_eq!((*ace).Header.AceType, 0); // ACCESS_ALLOWED_ACE_TYPE
            assert_eq!((*ace).Header.AceFlags, 0); // No inherited or inherit-only ACEs.
            let sid = PSID(std::ptr::addr_of_mut!((*ace).SidStart).cast());
            let role = expected
                .iter()
                .position(|(expected_sid, _)| EqualSid(sid, *expected_sid).is_ok())
                .expect("unexpected trustee in the created pipe's DACL");
            assert!(!seen[role], "duplicate trustee in the created pipe's DACL");
            assert_eq!((*ace).Mask, expected[role].1);
            seen[role] = true;
        }
    }
    assert!(seen.into_iter().all(|present| present));
}

#[test]
fn restricted_thread_token_is_reverted_even_when_a_test_panics() {
    let result = std::panic::catch_unwind(|| {
        without_admin_rights(|| panic!("synthetic test failure"));
    });
    assert!(result.is_err());
    assert_no_thread_token();
}
