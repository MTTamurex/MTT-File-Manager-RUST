//! Retry ACL opening with backup/restore rights on a temporary thread token.
//! Never change the GUI token, ownership, or the process-wide privilege state.

use std::marker::PhantomData;
use std::path::Path;
use std::rc::Rc;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    GetLastError, SetLastError, ERROR_NO_TOKEN, HANDLE, LUID, WIN32_ERROR,
};
use windows::Win32::Security::*;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken, SetThreadToken,
};

use super::super::owned_handle::OwnedHandle;
use super::{acl, diagnostics, os_error_code};

pub(super) fn grant_access(path: &Path, sid: &str) -> Result<(), u32> {
    match acl::grant_access_detailed(path, sid) {
        Err(code) if diagnostics::decode(code) == ("open_directory", 5) => {
            // The first attempt did not open the directory or change its ACL.
            // This is only called by the authenticated, elevated helper.
            let _scope = BackupRestoreScope::enable()
                .map_err(|code| diagnostics::HelperStage::BackupRestorePrivileges.encode(code))?;
            acl::grant_access_detailed(path, sid)
        }
        result => result,
    }
}

struct BackupRestoreScope {
    previous: Option<OwnedHandle>,
    // Thread impersonation must always be undone on the same thread.
    _thread_bound: PhantomData<Rc<()>>,
}

impl BackupRestoreScope {
    fn enable() -> Result<Self, u32> {
        Self::with_attributes(SE_PRIVILEGE_ENABLED)
    }

    fn with_attributes(attributes: TOKEN_PRIVILEGES_ATTRIBUTES) -> Result<Self, u32> {
        unsafe {
            let mut previous = HANDLE::default();
            let previous = match OpenThreadToken(
                GetCurrentThread(),
                TOKEN_QUERY | TOKEN_IMPERSONATE,
                true,
                &mut previous,
            ) {
                Ok(()) => Some(OwnedHandle::new(previous).ok_or(6u32)?),
                Err(error) if error.code() == ERROR_NO_TOKEN.to_hresult() => None,
                Err(error) => return Err(os_error_code(error)),
            };
            let mut process = HANDLE::default();
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY | TOKEN_DUPLICATE,
                &mut process,
            )
            .map_err(os_error_code)?;
            let process = OwnedHandle::new(process).ok_or(6u32)?;
            let mut duplicate = HANDLE::default();
            DuplicateTokenEx(
                process.as_raw(),
                TOKEN_QUERY | TOKEN_ADJUST_PRIVILEGES | TOKEN_IMPERSONATE,
                None,
                SecurityImpersonation,
                TokenImpersonation,
                &mut duplicate,
            )
            .map_err(os_error_code)?;
            let duplicate = OwnedHandle::new(duplicate).ok_or(6u32)?;
            for name in [SE_BACKUP_NAME, SE_RESTORE_NAME] {
                let mut luid = LUID::default();
                LookupPrivilegeValueW(PCWSTR::null(), name, &mut luid).map_err(os_error_code)?;
                let privileges = TOKEN_PRIVILEGES {
                    PrivilegeCount: 1,
                    Privileges: [LUID_AND_ATTRIBUTES {
                        Luid: luid,
                        Attributes: attributes,
                    }],
                };
                SetLastError(WIN32_ERROR(0));
                AdjustTokenPrivileges(duplicate.as_raw(), false, Some(&privileges), 0, None, None)
                    .map_err(os_error_code)?;
                // TRUE also means partial success: missing privileges must fail closed.
                let status = GetLastError();
                if status.0 != 0 {
                    return Err(status.0);
                }
            }
            SetThreadToken(None, Some(duplicate.as_raw())).map_err(os_error_code)?;
            Ok(Self {
                previous,
                _thread_bound: PhantomData,
            })
        }
    }
}

impl Drop for BackupRestoreScope {
    fn drop(&mut self) {
        if unsafe { SetThreadToken(None, self.previous.as_ref().map(OwnedHandle::as_raw)) }.is_err()
        {
            // Never continue with backup/restore privileges after a failed restore.
            std::process::abort();
        }
    }
}

#[cfg(test)]
#[path = "privileges_tests.rs"]
mod tests;
