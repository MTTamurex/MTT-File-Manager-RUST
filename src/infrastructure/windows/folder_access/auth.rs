use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Globalization::{CompareStringOrdinal, CSTR_EQUAL};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Pipes::{
    GetNamedPipeClientProcessId, GetNamedPipeClientSessionId, GetNamedPipeServerProcessId,
};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::*;

use super::super::owned_handle::OwnedHandle;
use super::{os_error_code, LocalAllocation};

pub(super) fn session_id(pid: u32) -> Result<u32, u32> {
    let mut session = 0;
    unsafe { ProcessIdToSessionId(pid, &mut session) }.map_err(os_error_code)?;
    Ok(session)
}

pub(super) fn require_elevated() -> Result<(), u32> {
    let token = process_token()?;
    let mut elevation = TOKEN_ELEVATION::default();
    let mut size = 0;
    unsafe {
        GetTokenInformation(
            token.as_raw(),
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        )
    }
    .map_err(os_error_code)?;
    if elevation.TokenIsElevated == 0 {
        return Err(5);
    }
    Ok(())
}

fn process_token() -> Result<OwnedHandle, u32> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(os_error_code)?;
    OwnedHandle::new(token).ok_or(6)
}

fn token_user_sid(token: HANDLE) -> Result<String, u32> {
    let mut needed = 0;
    unsafe {
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
    }
    if needed == 0 || needed > 4096 {
        return Err(5);
    }
    // TOKEN_USER contains pointers: supply a properly aligned native buffer.
    let mut buffer = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
        .map_err(os_error_code)?;
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        if !IsValidSid(user.User.Sid).as_bool() {
            return Err(5);
        }
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text).map_err(os_error_code)?;
        let _allocation = LocalAllocation(text.0.cast());
        text.to_string().map_err(|_| 5)
    }
}

#[cfg(test)]
pub(super) fn current_user_sid() -> Result<String, u32> {
    token_user_sid(process_token()?.as_raw())
}

fn file_identity(path: &Path) -> Result<(u32, u32, u32), u32> {
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let file = CreateFileW(
            PCWSTR(path.as_ptr()),
            FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
        .map_err(os_error_code)?;
        let file = OwnedHandle::new(file).ok_or(6u32)?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        GetFileInformationByHandle(file.as_raw(), &mut info).map_err(os_error_code)?;
        if info.nFileIndexHigh == 0 && info.nFileIndexLow == 0 {
            return Err(5); // Do not treat unavailable file identities as a match.
        }
        Ok((
            info.dwVolumeSerialNumber,
            info.nFileIndexHigh,
            info.nFileIndexLow,
        ))
    }
}

fn same_image(left: &Path, right: &Path) -> Result<bool, u32> {
    // Volume serials/file IDs alone are not globally unique across cloned or
    // virtual filesystems. Also require the same resolved executable location.
    let left = std::fs::canonicalize(left).map_err(|_| 5u32)?;
    let right = std::fs::canonicalize(right).map_err(|_| 5u32)?;
    let left_name: Vec<_> = left.as_os_str().encode_wide().collect();
    let right_name: Vec<_> = right.as_os_str().encode_wide().collect();
    if unsafe { CompareStringOrdinal(&left_name, &right_name, true) } != CSTR_EQUAL {
        return Ok(false);
    }
    Ok(file_identity(&left)? == file_identity(&right)?)
}

pub(super) struct Requester {
    process: OwnedHandle,
    pid: u32,
    session: u32,
}

impl Requester {
    pub fn open(pid: u32) -> Result<Self, u32> {
        if pid == 0 {
            return Err(5);
        }
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            .map_err(os_error_code)?;
        let process = OwnedHandle::new(process).ok_or(6u32)?;
        let session = session_id(pid)?;
        if session != session_id(std::process::id())? {
            return Err(5);
        }
        let mut image = vec![0u16; 32_768];
        let mut len = image.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                process.as_raw(),
                PROCESS_NAME_FORMAT(0),
                PWSTR(image.as_mut_ptr()),
                &mut len,
            )
        }
        .map_err(os_error_code)?;
        let image = OsString::from_wide(&image[..len as usize]);
        let own_image = std::env::current_exe().map_err(|_| 5u32)?;
        if !same_image(Path::new(&image), &own_image)? {
            return Err(5);
        }
        Ok(Self {
            process,
            pid,
            session,
        })
    }

    pub fn process(&self) -> HANDLE {
        self.process.as_raw()
    }

    pub fn verify_client(&self, pipe: HANDLE) -> Result<(), u32> {
        let (mut pid, mut session) = (0, 0);
        unsafe {
            GetNamedPipeClientProcessId(pipe, &mut pid).map_err(os_error_code)?;
            GetNamedPipeClientSessionId(pipe, &mut session).map_err(os_error_code)?;
        }
        if pid != self.pid || session != self.session {
            return Err(5);
        }
        Ok(())
    }

    pub fn client_sid(&self, pipe: HANDLE) -> Result<String, u32> {
        self.verify_client(pipe)?;
        // Read the connecting thread's Windows identity, not the helper's admin
        // account. Identification-only SQOS suffices and prevents elevation theft.
        unsafe { windows::Win32::System::Pipes::ImpersonateNamedPipeClient(pipe) }
            .map_err(os_error_code)?;
        struct Revert;
        impl Drop for Revert {
            fn drop(&mut self) {
                if unsafe { RevertToSelf() }.is_err() {
                    std::process::abort(); // Never continue under an unknown token.
                }
            }
        }
        let _revert = Revert;
        let mut token = HANDLE::default();
        unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token) }
            .map_err(os_error_code)?;
        let token = OwnedHandle::new(token).ok_or(6u32)?;
        let sid = token_user_sid(token.as_raw())?;
        if matches!(
            sid.as_str(),
            "S-1-5-18" | "S-1-5-19" | "S-1-5-20" | "S-1-5-7"
        ) {
            return Err(5);
        }
        Ok(sid)
    }
}

pub(super) fn verify_server(pipe: HANDLE, launched_process: HANDLE) -> Result<(), u32> {
    let expected = unsafe { GetProcessId(launched_process) };
    let mut actual = 0;
    unsafe { GetNamedPipeServerProcessId(pipe, &mut actual) }.map_err(os_error_code)?;
    if expected == 0 || actual != expected {
        return Err(5);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::same_image;

    #[test]
    fn image_checks_resolve_aliases_but_reject_same_name_in_another_location() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("Pasta 日本語");
        let second = directory.path().join("Other");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        std::fs::write(first.join("app.exe"), b"sample").unwrap();
        std::fs::write(second.join("app.exe"), b"sample").unwrap();
        std::fs::create_dir(first.join("nested")).unwrap();
        assert!(same_image(&first.join("app.exe"), &first.join(r"nested\..\app.exe")).unwrap());
        assert!(!same_image(&first.join("app.exe"), &second.join("app.exe")).unwrap());
    }
}
