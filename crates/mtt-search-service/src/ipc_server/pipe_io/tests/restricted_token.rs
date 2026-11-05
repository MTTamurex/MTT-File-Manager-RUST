//! Temporarily disable administrator membership on this test thread only.
//! No accounts, process-wide token settings or persistent ACLs are changed.

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use windows::Win32::Foundation::{ERROR_NO_TOKEN, HANDLE};
use windows::Win32::Security::{
    CheckTokenMembership, CreateRestrictedToken, ImpersonateLoggedOnUser, RevertToSelf,
    DISABLE_MAX_PRIVILEGE, SID_AND_ATTRIBUTES, TOKEN_DUPLICATE, TOKEN_QUERY,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken,
};

use super::super::SidGuard;

pub(super) fn assert_no_thread_token() {
    let mut token = HANDLE::default();
    match unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token) } {
        Err(error) => assert_eq!(error.code(), ERROR_NO_TOKEN.to_hresult()),
        Ok(()) => {
            let _token = unsafe { OwnedHandle::from_raw_handle(token.0) };
            panic!("test thread unexpectedly has an impersonation token");
        }
    }
}

pub(super) fn without_admin_rights<T>(operation: impl FnOnce() -> T) -> T {
    assert_no_thread_token();
    let admins = SidGuard::allocate(2, 32, 544).unwrap();
    let mut process_token = HANDLE::default();
    unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_DUPLICATE | TOKEN_QUERY,
            &mut process_token,
        )
        .unwrap();
    }
    let process_token = unsafe { OwnedHandle::from_raw_handle(process_token.0) };
    let disabled = [SID_AND_ATTRIBUTES {
        Sid: admins.0,
        Attributes: 0,
    }];
    let mut restricted = HANDLE::default();
    unsafe {
        CreateRestrictedToken(
            HANDLE(process_token.as_raw_handle()),
            DISABLE_MAX_PRIVILEGE,
            Some(&disabled),
            None,
            None,
            &mut restricted,
        )
        .unwrap();
    }
    let restricted = unsafe { OwnedHandle::from_raw_handle(restricted.0) };

    struct RevertGuard;
    impl Drop for RevertGuard {
        fn drop(&mut self) {
            if unsafe { RevertToSelf() }.is_err() {
                // Never let a test continue or return to the runner with an
                // unexpected impersonation context, even during unwinding.
                std::process::abort();
            }
        }
    }

    unsafe { ImpersonateLoggedOnUser(HANDLE(restricted.as_raw_handle())) }.unwrap();
    let guard = RevertGuard;
    let mut member = windows::core::BOOL::default();
    unsafe { CheckTokenMembership(None, admins.0, &mut member) }.unwrap();
    assert!(
        !member.as_bool(),
        "administrator membership must be disabled"
    );
    let result = operation();
    drop(guard);
    assert_no_thread_token();
    result
}
