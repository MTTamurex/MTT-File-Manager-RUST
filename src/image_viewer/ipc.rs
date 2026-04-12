use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, GetLastError, HANDLE,
};
use windows::Win32::Security::{
    InitializeSecurityDescriptor, SetSecurityDescriptorDacl, PSECURITY_DESCRIPTOR,
    SECURITY_ATTRIBUTES,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_NONE, OPEN_EXISTING,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};

pub const IMAGE_VIEWER_PIPE_NAME: &str = r"\\.\pipe\MTTFileManagerImageViewer";

const PIPE_BUFFER_SIZE: u32 = 32 * 1024;
const MAX_PIPE_INSTANCES: u32 = 4;
const MAX_MESSAGE_BYTES: usize = 32 * 1024;
const CLIENT_RETRY_COUNT: usize = 8;
const CLIENT_RETRY_DELAY: Duration = Duration::from_millis(60);
const SERVER_RETRY_DELAY: Duration = Duration::from_millis(250);
const ERROR_PIPE_CONNECTED_CODE: u32 = 535;

pub fn send_open_request(path: &Path) -> Result<bool, String> {
    let payload = path.to_string_lossy().into_owned().into_bytes();
    if payload.len() > MAX_MESSAGE_BYTES {
        return Err(format!(
            "image path is too large for viewer IPC: {} bytes",
            payload.len()
        ));
    }

    let pipe_name_wide: Vec<u16> = IMAGE_VIEWER_PIPE_NAME
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    for attempt in 0..CLIENT_RETRY_COUNT {
        unsafe {
            match CreateFileW(
                PCWSTR(pipe_name_wide.as_ptr()),
                0x80000000 | 0x40000000,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            ) {
                Ok(pipe) => {
                    let result = write_message(pipe, &payload);
                    let _ = CloseHandle(pipe);
                    return result.map(|_| true);
                }
                Err(err) => {
                    let code = err.code();
                    if code == ERROR_FILE_NOT_FOUND.to_hresult()
                        || code == ERROR_PIPE_BUSY.to_hresult()
                    {
                        if attempt + 1 < CLIENT_RETRY_COUNT {
                            std::thread::sleep(CLIENT_RETRY_DELAY);
                            continue;
                        }
                        return Ok(false);
                    }

                    return Err(format!("failed to open image viewer IPC pipe: {}", err));
                }
            }
        }
    }

    Ok(false)
}

pub fn start_open_request_server() -> Receiver<PathBuf> {
    let (tx, rx) = mpsc::channel();

    std::thread::Builder::new()
        .name("image-viewer-ipc".into())
        .spawn(move || {
            loop {
                let pipe = match create_pipe() {
                    Ok(pipe) => pipe,
                    Err(err) => {
                        log::warn!("[IMAGE-VIEWER] failed to create IPC pipe: {}", err);
                        std::thread::sleep(SERVER_RETRY_DELAY);
                        continue;
                    }
                };

                let connected = unsafe {
                    ConnectNamedPipe(pipe, None).is_ok()
                        || GetLastError().0 == ERROR_PIPE_CONNECTED_CODE
                };

                if connected {
                    match read_message(pipe) {
                        Ok(path) => {
                            if tx.send(path).is_err() {
                                let _ = unsafe { DisconnectNamedPipe(pipe) };
                                let _ = unsafe { CloseHandle(pipe) };
                                break;
                            }
                        }
                        Err(err) => {
                            log::warn!("[IMAGE-VIEWER] failed to read IPC request: {}", err);
                        }
                    }
                }

                let _ = unsafe { DisconnectNamedPipe(pipe) };
                let _ = unsafe { CloseHandle(pipe) };
            }
        })
        .ok();

    rx
}

fn create_pipe() -> Result<HANDLE, String> {
    let pipe_name_wide: Vec<u16> = IMAGE_VIEWER_PIPE_NAME
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        // Build an explicit DACL granting access to BUILTIN\Users and SYSTEM only.
        // Without this, the pipe inherits the process default descriptor which may
        // be overly permissive and allow local DoS via repeated open_request spam.

        // BUILTIN\Users SID: S-1-5-32-545
        let mut sid_users = [0u8; 16];
        sid_users[0] = 1; // Revision
        sid_users[1] = 2; // SubAuthorityCount
        sid_users[7] = 5; // NT Authority
        sid_users[8..12].copy_from_slice(&32u32.to_le_bytes());
        sid_users[12..16].copy_from_slice(&545u32.to_le_bytes());

        // NT AUTHORITY\SYSTEM SID: S-1-5-18
        let mut sid_system = [0u8; 12];
        sid_system[0] = 1;
        sid_system[1] = 1;
        sid_system[7] = 5;
        sid_system[8..12].copy_from_slice(&18u32.to_le_bytes());

        let ace1_size = 8 + sid_users.len();   // 24
        let ace2_size = 8 + sid_system.len();   // 20
        let acl_size = 8 + ace1_size + ace2_size;

        let mut acl_buffer = vec![0u8; acl_size];
        acl_buffer[0] = 2; // ACL_REVISION
        acl_buffer[2..4].copy_from_slice(&(acl_size as u16).to_le_bytes());
        acl_buffer[4..6].copy_from_slice(&2u16.to_le_bytes()); // AceCount

        // Users: read+write access for pipe clients
        let access_mask_users: u32 = 0x0012019F;
        let access_mask_system: u32 = 0x001F01FF;

        let ace1_off = 8;
        acl_buffer[ace1_off] = 0; // ACCESS_ALLOWED_ACE_TYPE
        acl_buffer[ace1_off + 2..ace1_off + 4].copy_from_slice(&(ace1_size as u16).to_le_bytes());
        acl_buffer[ace1_off + 4..ace1_off + 8].copy_from_slice(&access_mask_users.to_le_bytes());
        acl_buffer[ace1_off + 8..ace1_off + 8 + sid_users.len()].copy_from_slice(&sid_users);

        let ace2_off = ace1_off + ace1_size;
        acl_buffer[ace2_off] = 0;
        acl_buffer[ace2_off + 2..ace2_off + 4].copy_from_slice(&(ace2_size as u16).to_le_bytes());
        acl_buffer[ace2_off + 4..ace2_off + 8].copy_from_slice(&access_mask_system.to_le_bytes());
        acl_buffer[ace2_off + 8..ace2_off + 8 + sid_system.len()].copy_from_slice(&sid_system);

        let mut sd_buffer = vec![0u8; 256];
        let sd_ptr = PSECURITY_DESCRIPTOR(sd_buffer.as_mut_ptr() as *mut _);
        InitializeSecurityDescriptor(sd_ptr, 1)
            .map_err(|e| format!("InitializeSecurityDescriptor: {}", e))?;

        let acl_ptr = acl_buffer.as_ptr() as *const windows::Win32::Security::ACL;
        SetSecurityDescriptorDacl(sd_ptr, true, Some(acl_ptr), false)
            .map_err(|e| format!("SetSecurityDescriptorDacl: {}", e))?;

        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd_ptr.0,
            bInheritHandle: false.into(),
        };

        // SEC: FILE_FLAG_FIRST_PIPE_INSTANCE (0x00080000) prevents pipe squatting
        // by failing if a pipe with this name already exists from a rogue process.
        // PIPE_ACCESS_DUPLEX = 0x00000003.
        const PIPE_OPEN_MODE: u32 = 0x00000003 | 0x00080000;

        let pipe = CreateNamedPipeW(
            PCWSTR(pipe_name_wide.as_ptr()),
            windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(PIPE_OPEN_MODE),
            PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            MAX_PIPE_INSTANCES,
            PIPE_BUFFER_SIZE,
            PIPE_BUFFER_SIZE,
            0,
            Some(&sa as *const _),
        );

        if pipe.is_invalid() {
            return Err(format!("CreateNamedPipeW failed: {:?}", GetLastError()));
        }

        Ok(pipe)
    }
}

fn write_message(pipe: HANDLE, payload: &[u8]) -> Result<(), String> {
    let len = u32::try_from(payload.len()).map_err(|_| "payload too large".to_string())?;
    write_all(pipe, &len.to_le_bytes())?;
    write_all(pipe, payload)
}

fn read_message(pipe: HANDLE) -> Result<PathBuf, String> {
    let mut len_buf = [0u8; 4];
    read_exact(pipe, &mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len == 0 || len > MAX_MESSAGE_BYTES {
        return Err(format!("invalid IPC payload length: {}", len));
    }

    let mut payload = vec![0u8; len];
    read_exact(pipe, &mut payload)?;
    let path = String::from_utf8(payload).map_err(|e| format!("invalid UTF-8 path: {}", e))?;
    Ok(PathBuf::from(path))
}

fn write_all(pipe: HANDLE, mut data: &[u8]) -> Result<(), String> {
    while !data.is_empty() {
        let mut bytes_written = 0u32;
        unsafe {
            WriteFile(pipe, Some(data), Some(&mut bytes_written), None)
                .map_err(|e| format!("WriteFile failed: {}", e))?;
        }

        if bytes_written == 0 {
            return Err("pipe write returned 0 bytes".to_string());
        }

        data = &data[bytes_written as usize..];
    }

    Ok(())
}

fn read_exact(pipe: HANDLE, mut out: &mut [u8]) -> Result<(), String> {
    while !out.is_empty() {
        let mut bytes_read = 0u32;
        unsafe {
            ReadFile(pipe, Some(out), Some(&mut bytes_read), None)
                .map_err(|e| format!("ReadFile failed: {}", e))?;
        }

        if bytes_read == 0 {
            return Err("pipe closed before full message was read".to_string());
        }

        let (_, rest) = out.split_at_mut(bytes_read as usize);
        out = rest;
    }

    Ok(())
}