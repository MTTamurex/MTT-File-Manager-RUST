//! Deny data/list access only on objects inside a test-owned temporary directory.
//! Keep an open WRITE_DAC handle and restore the original ACL before cleanup.

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Component, Path};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{LocalFree, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::{
    GetSecurityInfo, SetEntriesInAclW, SetSecurityInfo, DENY_ACCESS, EXPLICIT_ACCESS_W,
    SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
};
use windows::Win32::Security::{ACE_FLAGS, ACL, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};

use super::super::SidGuard;

struct Allocation(HLOCAL);

impl Drop for Allocation {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(self.0));
        }
    }
}

pub(super) struct DenyRead {
    handle: OwnedHandle,
    original_acl: *mut ACL,
    _descriptor: Allocation,
}

impl DenyRead {
    pub fn new(root: &tempfile::TempDir, relative: &str) -> Self {
        let relative = Path::new(relative);
        assert!(relative.components().next().is_some());
        assert!(relative
            .components()
            .all(|part| matches!(part, Component::Normal(_))));
        let path = root.path().join(relative);
        assert!(!path.symlink_metadata().unwrap().file_type().is_symlink());
        let wide: Vec<u16> = path
            .to_str()
            .unwrap()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                0x0002_0000 | 0x0004_0000, // READ_CONTROL | WRITE_DAC, not data access.
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                None,
            )
        }
        .expect("open synthetic ACL fixture");
        let handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let mut original_acl = std::ptr::null_mut();
        let status = unsafe {
            GetSecurityInfo(
                HANDLE(handle.as_raw_handle()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut original_acl),
                None,
                Some(&mut descriptor),
            )
        };
        assert_eq!(status.0, 0, "read synthetic ACL");
        let guard = Self {
            handle,
            original_acl,
            _descriptor: Allocation(HLOCAL(descriptor.0)),
        };
        assert!(!guard.original_acl.is_null());
        let users = SidGuard::authenticated_users().unwrap();
        let entries = [EXPLICIT_ACCESS_W {
            grfAccessPermissions: 1, // FILE_READ_DATA / FILE_LIST_DIRECTORY only.
            grfAccessMode: DENY_ACCESS,
            grfInheritance: ACE_FLAGS(0), // No inherited deny on descendants.
            Trustee: TRUSTEE_W {
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                ptstrName: users.as_trustee_name(),
                ..Default::default()
            },
        }];
        let mut acl = std::ptr::null_mut();
        let status = unsafe { SetEntriesInAclW(Some(&entries), Some(original_acl), &mut acl) };
        assert_eq!(status.0, 0, "build synthetic deny ACL");
        let _acl = Allocation(HLOCAL(acl.cast()));
        let status = unsafe {
            SetSecurityInfo(
                HANDLE(guard.handle.as_raw_handle()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(acl),
                None,
            )
        };
        assert_eq!(status.0, 0, "apply synthetic deny ACL");
        guard
    }
}

impl Drop for DenyRead {
    fn drop(&mut self) {
        let status = unsafe {
            SetSecurityInfo(
                HANDLE(self.handle.as_raw_handle()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(self.original_acl),
                None,
            )
        };
        if status.0 != 0 {
            // Do not silently leave a changed ACL behind, including on unwind.
            eprintln!("synthetic ACL restoration failed: {}", status.0);
            std::process::abort();
        }
    }
}
