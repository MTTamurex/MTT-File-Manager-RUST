use super::*;
use windows::Win32::Storage::FileSystem::FILE_LIST_DIRECTORY;

struct RestoreAcl {
    name: Vec<u16>,
    _descriptor: LocalAllocation,
    acl: *mut ACL,
}

impl Drop for RestoreAcl {
    fn drop(&mut self) {
        // Restore before TempDir cleanup, including when an assertion panics.
        unsafe {
            let _ = SetNamedSecurityInfoW(
                PCWSTR(self.name.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(self.acl),
                None,
            );
        }
    }
}

#[test]
fn successful_acl_write_does_not_hide_an_effective_access_denial() {
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(verify_access(directory.path()), Ok(()));
    let name: Vec<u16> = directory
        .path()
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let sid_text = current_user_sid().unwrap();
    let sid_wide: Vec<u16> = sid_text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let mut acl = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        assert_eq!(
            GetNamedSecurityInfoW(
                PCWSTR(name.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut acl),
                None,
                &mut descriptor,
            )
            .0,
            0
        );
        let restore = RestoreAcl {
            name,
            _descriptor: LocalAllocation(descriptor.0),
            acl,
        };
        let mut sid = PSID::default();
        ConvertStringSidToSidW(PCWSTR(sid_wide.as_ptr()), &mut sid).unwrap();
        let _sid = LocalAllocation(sid.0);
        let deny = EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_LIST_DIRECTORY.0,
            grfAccessMode: DENY_ACCESS,
            Trustee: TRUSTEE_W {
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_USER,
                ptstrName: PWSTR(sid.0.cast()),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut denied_acl = std::ptr::null_mut();
        assert_eq!(
            SetEntriesInAclW(Some(&[deny]), Some(acl), &mut denied_acl).0,
            0
        );
        let _denied_acl = LocalAllocation(denied_acl.cast());
        assert_eq!(
            SetNamedSecurityInfoW(
                PCWSTR(restore.name.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(denied_acl),
                None,
            )
            .0,
            0
        );

        assert_eq!(verify_access(directory.path()), Err(AccessError::Failed(5)));
        grant_access(directory.path(), &sid_text).unwrap();
        assert_eq!(verify_access(directory.path()), Err(AccessError::Failed(5)));
        drop(restore);
    }
    assert_eq!(verify_access(directory.path()), Ok(()));
}

#[test]
fn helper_reports_native_errors_for_missing_directories_and_files() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("file.txt");
    std::fs::write(&file, b"sample").unwrap();
    let sid = current_user_sid().unwrap();
    assert_eq!(run_helper(&file, &sid), 267);
    assert_eq!(run_helper(&directory.path().join("missing"), &sid), 2);
}

#[test]
fn hresult_errors_are_reported_as_native_windows_error_codes() {
    let error = windows::core::Error::from(windows::Win32::Foundation::ERROR_ACCESS_DENIED);
    assert_eq!(os_error_code(error), 5);
}
