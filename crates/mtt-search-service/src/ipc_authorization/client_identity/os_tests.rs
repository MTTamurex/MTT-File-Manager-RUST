//! Real Windows failures, not injected error strings. All peers are this test
//! process; none of these cases establish behavior under real LocalSystem.

use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::Path;

use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, HANDLE};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

use super::os_fixture::{assert_no_thread_token, with_restricting_sid, IdentityPipe};
use super::{
    inspect_client_with, inspect_file_manager_client, query_process_image_path,
    verify_sibling_directories, ClientIdentityError, PipeImpersonationGuard,
};

const IMPERSONATION: u32 = 0x00020000;
const IDENTIFICATION: u32 = 0x00010000;
const ANONYMOUS: u32 = 0;

fn temp_directory() -> tempfile::TempDir {
    tempfile::tempdir().unwrap_or_else(|_| panic!("cannot create synthetic identity fixture"))
}

fn assert_resolution_access_denied(path: &Path) {
    match std::fs::canonicalize(path) {
        Err(error) => assert_eq!(error.raw_os_error(), Some(5)),
        Ok(_) => panic!("restricting token did not deny fixture directory resolution"),
    }
}

fn assert_own_process_query_denied() {
    match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, std::process::id()) } {
        Err(error) => assert_eq!(error.code(), ERROR_ACCESS_DENIED.to_hresult()),
        Ok(handle) => {
            let _handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
            panic!("restricting token did not produce the required real access denial");
        }
    }
}

fn assert_snapshot_rejected(result: Result<(), ClientIdentityError>) {
    assert!(matches!(result, Err(ClientIdentityError::Rejected(reason))
        if reason == "Client process snapshot identifies a different executable basename"));
}

#[test]
fn own_process_image_can_be_queried_without_restricting_token() {
    assert_no_thread_token();
    assert!(query_process_image_path(std::process::id()).is_ok());
    let pipe = IdentityPipe::new(IMPERSONATION);
    assert!(matches!(
        inspect_file_manager_client(pipe.raw()),
        Err(ClientIdentityError::Rejected(_))
    ));
    assert_no_thread_token();
}

#[test]
fn real_primary_access_denial_is_recovered_by_pipe_impersonation() {
    let pipe = IdentityPipe::new(IMPERSONATION);
    let result = with_restricting_sid(|| {
        assert_own_process_query_denied();
        assert_eq!(pipe.client_pid(), std::process::id());
        inspect_file_manager_client(pipe.raw())
    });
    // The image query succeeds on retry, but the real test executable is not
    // MTT. Rejecting it is correct; successful retry is not an authorization grant.
    assert!(matches!(result, Err(ClientIdentityError::Rejected(_))));
    assert_no_thread_token();
}

#[test]
fn real_primary_denial_and_anonymous_client_are_rejected_by_snapshot() {
    let pipe = IdentityPipe::new(ANONYMOUS);
    let result = with_restricting_sid(|| {
        assert_own_process_query_denied();
        assert_eq!(pipe.client_pid(), std::process::id());
        inspect_file_manager_client(pipe.raw())
    });
    assert_snapshot_rejected(result);
    assert_no_thread_token();
}

#[test]
fn real_primary_denial_and_identification_client_are_rejected_by_snapshot() {
    let pipe = IdentityPipe::new(IDENTIFICATION);
    let result = with_restricting_sid(|| {
        assert_own_process_query_denied();
        inspect_file_manager_client(pipe.raw())
    });
    assert_snapshot_rejected(result);
    assert_no_thread_token();
}

#[test]
fn actual_impersonation_api_failure_is_not_treated_as_a_trusted_client() {
    let result = inspect_client_with(
        || {
            with_restricting_sid(|| {
                assert_own_process_query_denied();
                query_process_image_path(std::process::id())
            })
        },
        || {
            let _guard = PipeImpersonationGuard::new(HANDLE::default())?;
            panic!("an invalid handle must not permit impersonation")
        },
        || panic!("unavailable image must not reach sibling checks"),
        |_, _| panic!("unavailable image must not retry directories"),
        || Err("synthetic inconclusive snapshot"),
    );
    match result {
        Err(ClientIdentityError::Unavailable(reason)) => {
            assert!(reason.contains("ImpersonateNamedPipeClient failed:"));
        }
        _ => panic!("actual impersonation failure should leave identity unavailable"),
    }
    assert_no_thread_token();
}

#[test]
fn actual_directory_access_denial_is_unavailable_for_different_paths() {
    let root = temp_directory();
    let other = root.path().join("other");
    std::fs::create_dir(&other).unwrap();
    assert!(std::fs::canonicalize(root.path()).is_ok());
    assert!(std::fs::canonicalize(&other).is_ok());
    with_restricting_sid(|| {
        for path in [root.path(), other.as_path()] {
            assert_resolution_access_denied(path);
        }
        assert!(matches!(
            verify_sibling_directories(root.path(), &other),
            Err(ClientIdentityError::Unavailable(_))
        ));
    });
    assert!(std::fs::canonicalize(root.path()).is_ok());
}

#[test]
fn existing_lexical_fallback_still_accepts_identical_paths_when_windows_denies_resolution() {
    let root = temp_directory();
    with_restricting_sid(|| {
        assert_resolution_access_denied(root.path());
        assert_eq!(verify_sibling_directories(root.path(), root.path()), Ok(()));
    });
}

#[test]
fn real_client_context_can_resolve_an_alias_that_the_primary_context_cannot() {
    let root = temp_directory();
    let alias = root.path().join(".");
    assert!(root.path().as_os_str() != alias.as_os_str());
    assert_eq!(verify_sibling_directories(root.path(), &alias), Ok(()));
    let pipe = IdentityPipe::new(IMPERSONATION);
    with_restricting_sid(|| {
        assert!(matches!(
            verify_sibling_directories(root.path(), &alias),
            Err(ClientIdentityError::Unavailable(_))
        ));
        let _guard = PipeImpersonationGuard::new(pipe.raw()).unwrap();
        assert_eq!(verify_sibling_directories(root.path(), &alias), Ok(()));
    });
    assert_no_thread_token();
}

#[test]
fn identification_qos_also_denies_the_legacy_filesystem_access_check() {
    let root = temp_directory();
    let path = root.path().to_str().unwrap();
    assert!(crate::ipc_authorization::current_client_can_read_path(path));
    let pipe = IdentityPipe::new(IDENTIFICATION);
    {
        let _guard = PipeImpersonationGuard::new(pipe.raw()).unwrap();
        assert!(query_process_image_path(std::process::id()).is_err());
        assert!(!crate::ipc_authorization::current_client_can_read_path(
            path
        ));
    }
    assert_no_thread_token();
    assert!(crate::ipc_authorization::current_client_can_read_path(path));
}

#[test]
fn restricting_thread_token_is_removed_on_unwind() {
    let action_started = std::cell::Cell::new(false);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_restricting_sid(|| {
            assert_own_process_query_denied();
            action_started.set(true);
            panic!("synthetic identity-test failure");
        })
    }));
    assert!(result.is_err());
    assert!(
        action_started.get(),
        "setup failure must not count as the expected panic"
    );
    assert_no_thread_token();
    assert!(query_process_image_path(std::process::id()).is_ok());
}
