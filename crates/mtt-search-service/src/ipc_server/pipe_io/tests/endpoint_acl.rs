//! Edit access only on objects inside a test-owned temporary directory.
//! Keep an open WRITE_DAC handle and restore the original ACL before cleanup.

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Component, Path};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{LocalFree, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::{
    GetSecurityInfo, SetEntriesInAclW, SetSecurityInfo, ACCESS_MODE, DENY_ACCESS,
    EXPLICIT_ACCESS_W, GRANT_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_WELL_KNOWN_GROUP,
    TRUSTEE_W,
};
use windows::Win32::Security::{ACE_FLAGS, ACL, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_GENERIC_READ, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
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
    _acl: FixtureAcl,
}

impl DenyRead {
    pub fn new(root: &tempfile::TempDir, relative: &str) -> Self {
        assert!(!relative.is_empty());
        Self {
            _acl: FixtureAcl::edit(
                root,
                relative,
                SidGuard::authenticated_users().unwrap(),
                DENY_ACCESS,
                1,
            ),
        }
    }
}

pub(super) struct RestrictedRead {
    _acl: FixtureAcl,
}

impl RestrictedRead {
    pub fn root(root: &tempfile::TempDir) -> Self {
        Self {
            _acl: FixtureAcl::edit(
                root,
                "",
                SidGuard::allocate(1, 12, 0).unwrap(),
                GRANT_ACCESS,
                FILE_GENERIC_READ.0,
            ),
        }
    }

    pub fn attributes(root: &tempfile::TempDir, relative: &str) -> Self {
        assert!(!relative.is_empty());
        Self {
            _acl: FixtureAcl::edit(
                root,
                relative,
                SidGuard::allocate(1, 12, 0).unwrap(),
                GRANT_ACCESS,
                FILE_READ_ATTRIBUTES.0,
            ),
        }
    }
}

struct FixtureAcl {
    handle: OwnedHandle,
    original_acl: *mut ACL,
    _descriptor: Allocation,
}

impl FixtureAcl {
    fn edit(
        root: &tempfile::TempDir,
        relative: &str,
        sid: SidGuard,
        mode: ACCESS_MODE,
        access: u32,
    ) -> Self {
        let relative = Path::new(relative);
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
        let entries = [EXPLICIT_ACCESS_W {
            grfAccessPermissions: access,
            grfAccessMode: mode,
            grfInheritance: ACE_FLAGS(0), // No inherited access change on descendants.
            Trustee: TRUSTEE_W {
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                ptstrName: sid.as_trustee_name(),
                ..Default::default()
            },
        }];
        let mut acl = std::ptr::null_mut();
        let status = unsafe { SetEntriesInAclW(Some(&entries), Some(original_acl), &mut acl) };
        assert_eq!(status.0, 0, "build synthetic fixture ACL");
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
        assert_eq!(status.0, 0, "apply synthetic fixture ACL");
        guard
    }
}

impl Drop for FixtureAcl {
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
