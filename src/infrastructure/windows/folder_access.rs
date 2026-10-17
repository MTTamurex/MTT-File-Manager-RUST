//! Explorer-style permanent access grants, performed by a UAC-elevated helper.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{LocalFree, ERROR_CANCELLED, HLOCAL, HWND, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS,
    SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

use super::owned_handle::OwnedHandle;

mod acl;
mod auth;
mod pipe;
mod protocol;

#[cfg(test)]
use acl::grant_access;

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

#[cfg(test)]
fn current_user_sid() -> Result<String, AccessError> {
    auth::current_user_sid().map_err(AccessError::Failed)
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
    let payload = protocol::encode_path(path).map_err(AccessError::Failed)?;
    let request = protocol::HelperRequest::new().map_err(AccessError::Failed)?;
    let exe = std::env::current_exe().map_err(|_| AccessError::Failed(1))?;
    let exe: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
    let verb: Vec<u16> = "runas".encode_utf16().chain(Some(0)).collect();
    let mut parameters: Vec<u16> = format!("{HELPER_FLAG} --ipc-v1 ").encode_utf16().collect();
    parameters.extend(quoted_argument(std::ffi::OsStr::new(&request.pipe_name)));
    parameters.extend(format!(" {}\0", request.requester_pid).encode_utf16());
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
        let deadline = std::time::Instant::now() + pipe::HANDSHAKE_TIMEOUT;
        let connection = (|| {
            let connection = pipe::connect(&request.pipe_name, deadline, process.as_raw())?;
            auth::verify_server(connection.as_raw(), process.as_raw())?;
            pipe::send_path(connection.as_raw(), &payload, deadline, process.as_raw())?;
            Ok::<_, u32>(connection)
        })();
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
        // Keep the client connection alive through helper exit so Windows can
        // identify its token after reading the request, including cross-account UAC.
        let _connection = connection.map_err(AccessError::Failed)?;
        if code == 0 {
            verify_access(path)
        } else {
            Err(AccessError::Failed(code))
        }
    }
}

/// Only versioned IPC coordinates are accepted, never a target path or SID.
pub fn run_helper(args: impl IntoIterator<Item = std::ffi::OsString>) -> i32 {
    let result = (|| {
        let request = protocol::HelperRequest::parse(args)?;
        auth::require_elevated()?;
        let requester = auth::Requester::open(request.requester_pid)?;
        let server = pipe::create_server(&request.pipe_name)?;
        let deadline = std::time::Instant::now() + pipe::HANDSHAKE_TIMEOUT;
        pipe::wait_for_client(server.as_raw(), deadline, requester.process())?;
        requester.verify_client(server.as_raw())?;
        let payload = pipe::receive_path(server.as_raw(), deadline, requester.process())?;
        let path = protocol::decode_path(&payload)?;
        let sid = requester.client_sid(server.as_raw())?;
        acl::grant_access(&path, &sid)
    })();
    result.err().unwrap_or(0) as i32
}

#[cfg(test)]
#[path = "folder_access_regression_tests.rs"]
mod regression_tests;

#[cfg(test)]
#[path = "folder_access_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "folder_access/ipc_tests.rs"]
mod ipc_tests;
