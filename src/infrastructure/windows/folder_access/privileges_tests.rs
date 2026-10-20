use super::*;
use std::os::windows::ffi::OsStrExt;
use windows::Win32::Security::Authorization::*;
use windows::Win32::Storage::FileSystem::*;

fn assert_no_thread_token() {
    let mut token = HANDLE::default();
    let error = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token) }
        .expect_err("temporary impersonation must not leak");
    assert_eq!(os_error_code(error), ERROR_NO_TOKEN.0);
}

fn assert_backup_restore_disabled() {
    unsafe {
        let mut token = HANDLE::default();
        OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token).unwrap();
        let token = OwnedHandle::new(token).unwrap();
        for name in [SE_BACKUP_NAME, SE_RESTORE_NAME] {
            let mut luid = LUID::default();
            LookupPrivilegeValueW(PCWSTR::null(), name, &mut luid).unwrap();
            let mut required = PRIVILEGE_SET {
                PrivilegeCount: 1,
                Control: 0,
                Privilege: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            let mut enabled = false.into();
            PrivilegeCheck(token.as_raw(), &mut required, &mut enabled).unwrap();
            assert!(
                !enabled.as_bool(),
                "backup/restore privilege leaked into baseline"
            );
        }
    }
}

#[test]
fn backup_restore_scope_never_leaves_a_thread_token_on_success_or_failure() {
    assert_no_thread_token();
    match BackupRestoreScope::enable() {
        Ok(scope) => {
            // A nested scope must restore the outer thread token, not remove it.
            drop(BackupRestoreScope::enable().unwrap());
            let mut token = HANDLE::default();
            unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token) }.unwrap();
            let _token = OwnedHandle::new(token).unwrap();
            drop(scope);
        }
        Err(code) => assert_eq!(code, 1300, "expected privileges absent from a normal token"),
    }
    assert_no_thread_token();
}

#[test]
fn ordinary_acl_grants_do_not_require_backup_restore_privileges() {
    let directory = tempfile::tempdir().unwrap();
    let sid = super::super::auth::current_user_sid().unwrap();
    grant_access(directory.path(), &sid).unwrap();
    assert_no_thread_token();
}

#[test]
fn missing_directory_does_not_trigger_privilege_retry() {
    let directory = tempfile::tempdir().unwrap();
    let sid = super::super::auth::current_user_sid().unwrap();
    assert_eq!(
        grant_access(&directory.path().join("missing"), &sid).map_err(diagnostics::decode),
        Err(("open_directory", 2)),
    );
    assert_no_thread_token();
}

struct RestoreAcl {
    directory: OwnedHandle,
    _descriptor: super::super::LocalAllocation,
    acl: *mut ACL,
}

impl Drop for RestoreAcl {
    fn drop(&mut self) {
        // The handle was opened before restricting the ACL and keeps WRITE_DAC.
        unsafe {
            let status = SetSecurityInfo(
                self.directory.as_raw(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | UNPROTECTED_DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(self.acl),
                None,
            );
            assert_eq!(status.0, 0, "temporary directory ACL must be restored");
        }
    }
}

#[test]
#[ignore = "requires an elevated test process with backup/restore privileges; modifies only a temporary directory"]
fn elevated_retry_opens_restricted_temporary_directory_without_changing_owner() {
    // Check prerequisites before changing any ACL. This test never requests UAC.
    drop(BackupRestoreScope::enable().expect("run this test from an elevated terminal"));
    // An elevated terminal may already have enabled these privileges. Make the
    // unprivileged open deterministic without changing the process token.
    let baseline = BackupRestoreScope::with_attributes(TOKEN_PRIVILEGES_ATTRIBUTES(0)).unwrap();
    assert_backup_restore_disabled();
    let directory = tempfile::tempdir().unwrap();
    let sid = super::super::auth::current_user_sid().unwrap();
    let name: Vec<u16> = directory
        .path()
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    unsafe {
        let handle = CreateFileW(
            PCWSTR(name.as_ptr()),
            READ_CONTROL.0 | WRITE_DAC.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
        .unwrap();
        let handle = OwnedHandle::new(handle).unwrap();
        let mut acl = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let mut owner = PSID::default();
        assert_eq!(
            GetSecurityInfo(
                handle.as_raw(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION,
                Some(&mut owner),
                None,
                Some(&mut acl),
                None,
                Some(&mut descriptor),
            )
            .0,
            0
        );
        let restore = RestoreAcl {
            directory: handle,
            _descriptor: super::super::LocalAllocation(descriptor.0),
            acl,
        };

        // OWNER RIGHTS suppresses the owner's implicit READ_CONTROL/WRITE_DAC.
        // This reproduces the failed open without copying a system folder's ACL.
        let sddl: Vec<u16> = format!("D:P(A;;0x80;;;OW)(A;;0x81;;;{sid})\0")
            .encode_utf16()
            .collect();
        let mut restricted = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut restricted,
            None,
        )
        .unwrap();
        let _restricted = super::super::LocalAllocation(restricted.0);
        let mut present = false.into();
        let mut defaulted = false.into();
        let mut restricted_acl = std::ptr::null_mut();
        GetSecurityDescriptorDacl(
            restricted,
            &mut present,
            &mut restricted_acl,
            &mut defaulted,
        )
        .unwrap();
        assert_eq!(
            SetSecurityInfo(
                restore.directory.as_raw(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(restricted_acl),
                None,
            )
            .0,
            0
        );

        assert_eq!(
            acl::grant_access_detailed(directory.path(), &sid).map_err(diagnostics::decode),
            Err(("open_directory", 5)),
        );
        grant_access(directory.path(), &sid).unwrap();
        // The retry must restore the baseline token, with both privileges disabled.
        // A fresh ordinary open must now succeed using the newly granted ACL.
        assert_backup_restore_disabled();
        acl::grant_access_detailed(directory.path(), &sid).unwrap();

        let mut after_owner = PSID::default();
        let mut after_descriptor = PSECURITY_DESCRIPTOR::default();
        assert_eq!(
            GetSecurityInfo(
                restore.directory.as_raw(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                Some(&mut after_owner),
                None,
                None,
                None,
                Some(&mut after_descriptor),
            )
            .0,
            0
        );
        let _after = super::super::LocalAllocation(after_descriptor.0);
        EqualSid(owner, after_owner).unwrap();
        drop(restore);
    }
    drop(baseline);
    assert_no_thread_token();
}
