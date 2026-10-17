use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Security::Authorization::*;
use windows::Win32::Security::{
    ACL, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, OBJECT_INHERIT_ACE,
    PSECURITY_DESCRIPTOR, PSID,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ALL_ACCESS,
    FILE_ATTRIBUTE_DIRECTORY, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING, READ_CONTROL, WRITE_DAC,
};

use super::super::owned_handle::OwnedHandle;
use super::{os_error_code, LocalAllocation};

/// Internal only: the helper obtains this SID from its authenticated pipe client.
/// Keep the directory pinned through the ACL read/merge/write operation.
pub(super) fn grant_access(path: &Path, sid_text: &str) -> Result<(), u32> {
    if !path.is_absolute()
        || path.as_os_str().encode_wide().any(|unit| unit == 0)
        || sid_text.contains('\0')
    {
        return Err(87);
    }
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let sid_text: Vec<u16> = sid_text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let handle = CreateFileW(
            PCWSTR(name.as_ptr()),
            READ_CONTROL.0 | WRITE_DAC.0 | FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
        .map_err(os_error_code)?;
        let directory = OwnedHandle::new(handle).ok_or(6u32)?;
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        GetFileInformationByHandle(directory.as_raw(), &mut information).map_err(os_error_code)?;
        if information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0 {
            return Err(267);
        }
        let mut sid = PSID::default();
        ConvertStringSidToSidW(PCWSTR(sid_text.as_ptr()), &mut sid).map_err(|_| 87u32)?;
        let _sid_allocation = LocalAllocation(sid.0);
        let mut old_acl = std::ptr::null_mut::<ACL>();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let status = GetSecurityInfo(
            directory.as_raw(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut old_acl),
            None,
            Some(&mut descriptor),
        );
        let _descriptor_allocation = LocalAllocation(descriptor.0);
        if status.0 != 0 {
            return Err(status.0);
        }
        // Preserve NULL DACLs, explicit denies, owners and inheritance protection.
        if old_acl.is_null() {
            return Ok(());
        }
        let entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_ALL_ACCESS.0,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: CONTAINER_INHERIT_ACE | OBJECT_INHERIT_ACE,
            Trustee: TRUSTEE_W {
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_USER,
                ptstrName: PWSTR(sid.0.cast()),
                ..Default::default()
            },
        };
        let mut new_acl = std::ptr::null_mut();
        let status = SetEntriesInAclW(Some(&[entry]), Some(old_acl), &mut new_acl);
        let _acl_allocation = LocalAllocation(new_acl.cast());
        if status.0 != 0 {
            return Err(status.0);
        }
        let status = SetSecurityInfo(
            directory.as_raw(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(new_acl),
            None,
        );
        if status.0 == 0 {
            Ok(())
        } else {
            Err(status.0)
        }
    }
}
