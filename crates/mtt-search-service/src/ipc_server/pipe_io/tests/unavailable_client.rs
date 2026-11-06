//! Full Impersonation SQOS with a restricting SID, on a unique fixture pipe.
//! Adds only that SID's client data rights, not rights to the installed pipe.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{FromRawHandle, OwnedHandle};

use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, HANDLE};
use windows::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, GRANT_ACCESS, TRUSTEE_IS_SID, TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
};
use windows::Win32::Storage::FileSystem::ReadFile;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

use crate::ipc_authorization::{
    inspect_file_manager_client, with_restricting_sid, ClientIdentityError, PipeImpersonationGuard,
};

use super::super::{create_pipe_with_entries, with_pipe_access_entries, SidGuard};
use super::fixture::{unique_name, Connection};

pub(super) fn connection() -> Connection {
    let name = unique_name();
    assert_ne!(name, mtt_search_protocol::PIPE_NAME);
    let restricted_code = SidGuard::allocate(1, 12, 0).unwrap();
    let raw = with_pipe_access_entries(|production| {
        let mut entries = production.to_vec();
        entries.push(EXPLICIT_ACCESS_W {
            grfAccessPermissions: mtt_search_protocol::PIPE_CLIENT_ACCESS,
            grfAccessMode: GRANT_ACCESS,
            Trustee: TRUSTEE_W {
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                ptstrName: restricted_code.as_trustee_name(),
                ..Default::default()
            },
            ..Default::default()
        });
        create_pipe_with_entries(&name, true, &entries)
    })
    .unwrap();
    let server = unsafe { OwnedHandle::from_raw_handle(raw.0) };
    let client = with_restricting_sid(|| {
        OpenOptions::new()
            .read(true)
            .write(true)
            .access_mode(mtt_search_protocol::PIPE_CLIENT_ACCESS)
            .custom_flags(0x00100000 | 0x00020000)
            .open(&name)
            .expect("connect restricted client to its own fixture pipe")
    });
    let mut connection = Connection::from_parts(server, client);
    // Establish a real last-read client context before the fixture's preflight
    // verifier. This byte is consumed here, not by production request framing.
    with_restricting_sid(|| connection.client.write_all(&[0x42]).unwrap());
    let mut byte = [0];
    let mut count = 0;
    unsafe { ReadFile(connection.server(), Some(&mut byte), Some(&mut count), None) }.unwrap();
    assert_eq!((byte, count), ([0x42], 1));
    connection
}

pub(super) fn assert_queries_denied_and_snapshot_rejects(pipe: HANDLE) {
    let assert_denied = || match unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, std::process::id())
    } {
        Err(error) => assert_eq!(error.code(), ERROR_ACCESS_DENIED.to_hresult()),
        Ok(handle) => {
            let _handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
            panic!("the fixture did not produce an actual process-query denial");
        }
    };
    with_restricting_sid(|| {
        assert_denied();
        let _guard = PipeImpersonationGuard::new(pipe).unwrap();
        assert_denied();
    });
    let result = with_restricting_sid(|| inspect_file_manager_client(pipe));
    assert!(matches!(result, Err(ClientIdentityError::Rejected(reason))
        if reason == "Client process snapshot identifies a different executable basename"));
}
