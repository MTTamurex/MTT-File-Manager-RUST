use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_NO_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
    ERROR_PIPE_LISTENING, HANDLE,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_FIRST_PIPE_INSTANCE,
    OPEN_EXISTING, PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::Pipes::*;
use windows::Win32::System::Threading::GetExitCodeProcess;

use super::super::owned_handle::OwnedHandle;
use super::protocol::MAX_PATH_BYTES;
use super::{os_error_code, LocalAllocation};

pub(super) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const CLIENT_ACCESS: u32 = 0x0012_019b; // File read/write rights minus FILE_CREATE_PIPE_INSTANCE

pub(super) fn process_exit_code(process: HANDLE) -> Result<Option<u32>, u32> {
    let mut code = 0;
    unsafe { GetExitCodeProcess(process, &mut code) }.map_err(os_error_code)?;
    Ok((code != 259).then_some(code)) // STILL_ACTIVE
}

fn check_pending(deadline: Instant, peer: HANDLE) -> Result<(), u32> {
    if Instant::now() >= deadline {
        return Err(1460);
    }
    if process_exit_code(peer)?.is_some() {
        return Err(109);
    }
    Ok(())
}

fn pause(deadline: Instant, peer: HANDLE) -> Result<(), u32> {
    check_pending(deadline, peer)?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(1460);
    }
    std::thread::sleep(remaining.min(POLL_INTERVAL));
    Ok(())
}

pub(super) fn create_server(name: &str) -> Result<OwnedHandle, u32> {
    // Cross-account UAC needs the original authenticated user to connect.
    // Permit data I/O only, not FILE_CREATE_PIPE_INSTANCE; reject remote clients.
    // A medium integrity label lets the ordinary GUI write to an elevated
    // helper without opening the channel to low-integrity sandboxed clients.
    let sddl: Vec<u16> = "D:P(A;;0x0012019b;;;AU)(A;;GA;;;SY)(A;;GA;;;BA)S:(ML;;NW;;;ME)\0"
        .encode_utf16()
        .collect();
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
        .map_err(os_error_code)?;
        let _descriptor = LocalAllocation(descriptor.0);
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        };
        let pipe = CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            128 * 1024,
            128 * 1024,
            0,
            Some(&attributes),
        );
        OwnedHandle::new(pipe).ok_or_else(|| os_error_code(windows::core::Error::from_thread()))
    }
}

pub(super) fn wait_for_client(pipe: HANDLE, deadline: Instant, peer: HANDLE) -> Result<(), u32> {
    loop {
        check_pending(deadline, peer)?;
        match unsafe { ConnectNamedPipe(pipe, None) } {
            Ok(()) => {}
            Err(error) => {
                let code = os_error_code(error);
                if code != ERROR_PIPE_LISTENING.0 && code != ERROR_PIPE_CONNECTED.0 {
                    return Err(code);
                }
            }
        }
        // In NOWAIT mode, ConnectNamedPipe may return success for entering the
        // listening state. Query the actual client instead of assuming a peer.
        let mut pid = 0;
        if unsafe { GetNamedPipeClientProcessId(pipe, &mut pid) }.is_ok() && pid != 0 {
            return Ok(());
        }
        pause(deadline, peer)?;
    }
}

pub(super) fn connect(name: &str, deadline: Instant, peer: HANDLE) -> Result<OwnedHandle, u32> {
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    loop {
        if Instant::now() >= deadline {
            return Err(1460);
        }
        match unsafe {
            CreateFileW(
                PCWSTR(name.as_ptr()),
                CLIENT_ACCESS,
                windows::Win32::Storage::FileSystem::FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(SECURITY_SQOS_PRESENT.0 | SECURITY_IDENTIFICATION.0),
                None,
            )
        } {
            Ok(pipe) => {
                let pipe = OwnedHandle::new(pipe).ok_or(6u32)?;
                unsafe { SetNamedPipeHandleState(pipe.as_raw(), Some(&PIPE_NOWAIT), None, None) }
                    .map_err(os_error_code)?;
                return Ok(pipe);
            }
            Err(error) => {
                let code = os_error_code(error);
                if code != ERROR_FILE_NOT_FOUND.0 && code != ERROR_PIPE_BUSY.0 {
                    return Err(code);
                }
                if let Some(code) = process_exit_code(peer)? {
                    return Err(if code == 0 { 109 } else { code });
                }
                pause(deadline, peer)?;
            }
        }
    }
}

fn write_all(pipe: HANDLE, mut bytes: &[u8], deadline: Instant, peer: HANDLE) -> Result<(), u32> {
    while !bytes.is_empty() {
        check_pending(deadline, peer)?;
        let mut written = 0;
        match unsafe { WriteFile(pipe, Some(bytes), Some(&mut written), None) } {
            Ok(()) if written > 0 => {
                bytes = &bytes[written as usize..];
            }
            Ok(()) => pause(deadline, peer)?,
            Err(error) if os_error_code(error.clone()) == ERROR_NO_DATA.0 => pause(deadline, peer)?,
            Err(error) => return Err(os_error_code(error)),
        }
    }
    Ok(())
}

fn read_exact(
    pipe: HANDLE,
    mut bytes: &mut [u8],
    deadline: Instant,
    peer: HANDLE,
) -> Result<(), u32> {
    while !bytes.is_empty() {
        check_pending(deadline, peer)?;
        let mut read = 0;
        match unsafe { ReadFile(pipe, Some(&mut *bytes), Some(&mut read), None) } {
            Ok(()) if read > 0 => {
                bytes = &mut bytes[read as usize..];
            }
            Ok(()) => pause(deadline, peer)?,
            Err(error) if os_error_code(error.clone()) == ERROR_NO_DATA.0 => pause(deadline, peer)?,
            Err(error) => return Err(os_error_code(error)),
        }
    }
    Ok(())
}

pub(super) fn send_path(
    pipe: HANDLE,
    bytes: &[u8],
    deadline: Instant,
    peer: HANDLE,
) -> Result<(), u32> {
    if bytes.is_empty() || bytes.len() > MAX_PATH_BYTES || !bytes.len().is_multiple_of(2) {
        return Err(87);
    }
    let mut frame = Vec::with_capacity(4 + bytes.len());
    frame.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    frame.extend_from_slice(bytes);
    write_all(pipe, &frame, deadline, peer)
}

pub(super) fn receive_path(pipe: HANDLE, deadline: Instant, peer: HANDLE) -> Result<Vec<u8>, u32> {
    let mut header = [0; 4];
    read_exact(pipe, &mut header, deadline, peer)?;
    let len = u32::from_le_bytes(header) as usize;
    if len == 0 || len > MAX_PATH_BYTES || !len.is_multiple_of(2) {
        return Err(87);
    }
    let mut bytes = vec![0; len];
    read_exact(pipe, &mut bytes, deadline, peer)?;
    Ok(bytes)
}
