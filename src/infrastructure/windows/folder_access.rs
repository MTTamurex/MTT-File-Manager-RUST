//! Explorer-style permanent access grants, performed by a UAC-elevated helper.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{LocalFree, ERROR_CANCELLED, HLOCAL, HWND, WAIT_OBJECT_0};
use windows::Win32::Security::Authorization::*;
use windows::Win32::Security::{ACL, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ALL_ACCESS,
    FILE_ATTRIBUTE_DIRECTORY, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING, READ_CONTROL, WRITE_DAC,
};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS,
    SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

use super::owned_handle::OwnedHandle;

pub const HELPER_FLAG: &str = "--grant-folder-access";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessError {
    Cancelled,
    Failed(u32),
}

fn os_error_code(error: windows::core::Error) -> u32 {
    let code = error.code().0 as u32;
    if code & 0xffff_0000 == 0x8007_0000 {
        code & 0xffff
    } else {
        code
    }
}

fn verify_access(path: &Path) -> Result<(), AccessError> {
    // Run in the original, non-elevated process. An allow ACE cannot override
    // explicit denies, and a successful ACL write alone does not prove access.
    std::fs::read_dir(path)
        .map(|_| ())
        .map_err(|error| AccessError::Failed(error.raw_os_error().unwrap_or(1) as u32))
}

struct LocalAllocation(*mut std::ffi::c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.0)));
        }
    }
}

/// Preserve the original user's SID across over-the-shoulder UAC elevation.
fn current_user_sid() -> Result<String, AccessError> {
    let (mut bytes, _) = crate::infrastructure::db_utils::get_current_user_sid_bytes()
        .ok_or(AccessError::Failed(1))?;
    let mut text = PWSTR::null();
    unsafe {
        ConvertSidToStringSidW(PSID(bytes.as_mut_ptr().cast()), &mut text)
            .map_err(|_| AccessError::Failed(1))?;
        let _allocation = LocalAllocation(text.0.cast());
        text.to_string().map_err(|_| AccessError::Failed(1))
    }
}

/// Quote an OS argument without a lossy UTF-16 conversion (including drive roots).
fn quoted_argument(value: &std::ffi::OsStr) -> Vec<u16> {
    let mut result = vec![b'"' as u16];
    let mut slashes = 0;
    for unit in value.encode_wide() {
        if unit == b'\\' as u16 {
            slashes += 1;
        } else {
            result.extend(std::iter::repeat_n(
                b'\\' as u16,
                slashes * if unit == b'"' as u16 { 2 } else { 1 },
            ));
            if unit == b'"' as u16 {
                result.push(b'\\' as u16);
            }
            result.push(unit);
            slashes = 0;
        }
    }
    result.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    result.push(b'"' as u16);
    result
}

/// Blocking: call only from a worker. Cancellation never changes permissions.
pub fn request_access(path: &Path, owner_window: Option<isize>) -> Result<(), AccessError> {
    let _com = super::com_scope::ComScope::sta();
    let sid = current_user_sid()?;
    let exe = std::env::current_exe().map_err(|_| AccessError::Failed(1))?;
    let exe: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
    let verb: Vec<u16> = "runas".encode_utf16().chain(Some(0)).collect();
    let mut parameters: Vec<u16> = format!("{HELPER_FLAG} ").encode_utf16().collect();
    parameters.extend(quoted_argument(path.as_os_str()));
    parameters.extend(format!(" {sid}\0").encode_utf16());
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
        hwnd: HWND(owner_window.unwrap_or(0) as *mut _),
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(exe.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        if let Err(error) = ShellExecuteExW(&mut info) {
            return Err(if error.code() == ERROR_CANCELLED.to_hresult() {
                AccessError::Cancelled
            } else {
                AccessError::Failed(os_error_code(error))
            });
        }
        let process = OwnedHandle::new(info.hProcess).ok_or(AccessError::Failed(1))?;
        // Keep the operation pending until the helper exits; a timeout would allow
        // another grant while the first process is still changing permissions.
        if WaitForSingleObject(process.as_raw(), INFINITE) != WAIT_OBJECT_0 {
            return Err(AccessError::Failed(os_error_code(
                windows::core::Error::from_thread(),
            )));
        }
        let mut code = 1;
        GetExitCodeProcess(process.as_raw(), &mut code)
            .map_err(|error| AccessError::Failed(os_error_code(error)))?;
        if code == 0 {
            verify_access(path)
        } else {
            Err(AccessError::Failed(code))
        }
    }
}

/// Add a full-control ACE while preserving existing ACEs, owner and inheritance.
/// Explicit denies and protected child ACLs are intentionally respected.
fn grant_access(path: &Path, sid_text: &str) -> Result<(), u32> {
    if !path.is_absolute()
        || path.as_os_str().encode_wide().any(|unit| unit == 0)
        || sid_text.contains('\0')
    {
        return Err(87);
    }
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let sid_text: Vec<u16> = sid_text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        // Pin the target through the whole read/merge/write operation. Using the
        // path again for SetNamedSecurityInfoW could update a replacement object.
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
        // A NULL DACL already permits everyone; replacing it would restrict access.
        if old_acl.is_null() {
            return Ok(());
        }
        let entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_ALL_ACCESS.0,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: windows::Win32::Security::ACE_FLAGS(3),
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

pub fn run_helper(path: &Path, sid: &str) -> i32 {
    grant_access(path, sid).err().unwrap_or(0) as i32
}

#[cfg(test)]
#[path = "folder_access_regression_tests.rs"]
mod regression_tests;

#[cfg(test)]
#[path = "folder_access_tests.rs"]
mod tests;
