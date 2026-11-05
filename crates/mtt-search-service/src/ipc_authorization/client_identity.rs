use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Pipes::GetNamedPipeClientProcessId;

use super::PipeImpersonationGuard;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClientIdentityError {
    /// An inspected image does not satisfy the existing MTT image policy.
    Rejected(String),
    /// Inspection failed; this is not evidence that the client is MTT.
    Unavailable(String),
}

impl std::fmt::Display for ClientIdentityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(reason) | Self::Unavailable(reason) => formatter.write_str(reason),
        }
    }
}

pub(crate) fn inspect_file_manager_client(pipe: HANDLE) -> Result<(), ClientIdentityError> {
    let mut client_pid = 0;
    unsafe {
        GetNamedPipeClientProcessId(pipe, &mut client_pid).map_err(|error| {
            ClientIdentityError::Unavailable(format!("GetNamedPipeClientProcessId failed: {error}"))
        })?;
    }
    if client_pid == 0 {
        return Err(ClientIdentityError::Unavailable("Client PID is 0".into()));
    }

    // Keep the LocalSystem compatibility retry: the service may be unable to
    // query an interactive process until it impersonates that pipe's client.
    // Only query failure permits retry; an inspected foreign image never does.
    inspect_client_with(
        || query_process_image_path(client_pid),
        || {
            let _guard = PipeImpersonationGuard::new(pipe)?;
            query_process_image_path(client_pid)
        },
        || std::env::current_exe().map_err(|error| format!("current_exe failed: {error}")),
    )
}

// Existing strictly protected endpoints must continue to deny BOTH error kinds.
pub(crate) fn trusted_file_manager_client(pipe: HANDLE) -> Result<(), String> {
    inspect_file_manager_client(pipe).map_err(|error| error.to_string())
}

fn inspect_client_with(
    primary: impl FnOnce() -> Result<String, String>,
    impersonated_retry: impl FnOnce() -> Result<String, String>,
    service_exe: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<(), ClientIdentityError> {
    let image = query_client_image_with_fallback(primary, impersonated_retry)
        .map_err(ClientIdentityError::Unavailable)?;
    let basename = Path::new(&image)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !basename.eq_ignore_ascii_case("mtt-file-manager.exe") {
        return Err(ClientIdentityError::Rejected(
            "Client image basename is '<redacted>', expected 'mtt-file-manager.exe'".into(),
        ));
    }

    let service_exe = service_exe().map_err(ClientIdentityError::Unavailable)?;
    let service_dir = service_exe.parent().ok_or_else(|| {
        ClientIdentityError::Unavailable("Service executable has no parent directory".into())
    })?;
    let client_dir = Path::new(&image).parent().ok_or_else(|| {
        ClientIdentityError::Rejected("Client executable has no parent directory".into())
    })?;
    verify_sibling_directories(service_dir, client_dir)
}

fn query_process_image_path(client_pid: u32) -> Result<String, String> {
    use windows::core::PWSTR;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, client_pid) }
        .map_err(|error| format!("OpenProcess(client pid {client_pid}) failed: {error}"))?;
    struct ProcessGuard(HANDLE);
    impl Drop for ProcessGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
    let _guard = ProcessGuard(process);
    let mut buffer = [0u16; 4096];
    let mut length = buffer.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
        .map_err(|error| {
            format!("QueryFullProcessImageNameW(client pid {client_pid}) failed: {error}")
        })?;
    }
    Ok(String::from_utf16_lossy(&buffer[..length as usize]))
}

fn query_client_image_with_fallback(
    primary: impl FnOnce() -> Result<String, String>,
    impersonated_retry: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    match primary() {
        Ok(image) => Ok(image),
        Err(primary_error) => impersonated_retry().map_err(|retry_error| {
            format!("client image query failed as service: {primary_error}; impersonated retry failed: {retry_error}")
        }),
    }
}

fn verify_sibling_directories(left: &Path, right: &Path) -> Result<(), ClientIdentityError> {
    let resolved_left = std::fs::canonicalize(left);
    let resolved_right = std::fs::canonicalize(right);
    let compared_left = resolved_left.as_deref().unwrap_or(left);
    let compared_right = resolved_right.as_deref().unwrap_or(right);
    // Preserve the existing successful comparison, including its lexical
    // fallback. But a mismatch with failed resolution is NOT a proven foreign
    // directory: it may be an inaccessible alias of the same installation.
    if compared_left
        .to_string_lossy()
        .eq_ignore_ascii_case(&compared_right.to_string_lossy())
    {
        Ok(())
    } else if resolved_left.is_err() || resolved_right.is_err() {
        Err(ClientIdentityError::Unavailable(
            "Cannot resolve client and service directories for identity verification".into(),
        ))
    } else {
        Err(ClientIdentityError::Rejected(
            "Client executable is not in the service directory".into(),
        ))
    }
}

#[cfg(test)]
mod tests;
