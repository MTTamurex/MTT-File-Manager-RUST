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
const CLIENT_INITIAL_RETRY_DELAY: Duration = Duration::from_millis(20);
const CLIENT_MAX_RETRY_DELAY: Duration = Duration::from_millis(500);
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
    let mut retry_delay = CLIENT_INITIAL_RETRY_DELAY;

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
                    if code == ERROR_FILE_NOT_FOUND.to_hresult() {
                        // Pipe does not exist — no viewer instance is running.
                        // Return immediately instead of retrying; there is
                        // nothing to connect to and waiting just delays the
                        // process spawn.
                        return Ok(false);
                    }
                    if code == ERROR_PIPE_BUSY.to_hresult() {
                        // Pipe exists but all instances are busy — the viewer
                        // is running, so retry with backoff.
                        if attempt + 1 < CLIENT_RETRY_COUNT {
                            std::thread::sleep(retry_delay);
                            retry_delay = retry_delay
                                .checked_mul(2)
                                .unwrap_or(CLIENT_MAX_RETRY_DELAY)
                                .min(CLIENT_MAX_RETRY_DELAY);
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
            log::info!(
                "[IMAGE-VIEWER] IPC server thread starting pid={} pipe='{}'",
                std::process::id(),
                IMAGE_VIEWER_PIPE_NAME
            );
            let pipe = match create_pipe() {
                Ok(p) => p,
                Err(err) => {
                    log::error!("[IMAGE-VIEWER] failed to create initial IPC pipe: {}", err);
                    return;
                }
            };

            loop {
                let connected = unsafe {
                    ConnectNamedPipe(pipe, None).is_ok()
                        || GetLastError().0 == ERROR_PIPE_CONNECTED_CODE
                };

                if connected {
                    match read_message(pipe) {
                        Ok(path) => {
                            log::info!(
                                "[IMAGE-VIEWER] IPC open request received pid={} path='{}'",
                                std::process::id(),
                                path.display()
                            );
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

                // Reuse the same pipe instance to avoid a race window between
                // CloseHandle and CreateNamedPipeW where a rogue process could
                // squat the pipe name.
                let _ = unsafe { DisconnectNamedPipe(pipe) };
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
        // SEC: Restrict the pipe DACL to the CURRENT USER's SID only (plus SYSTEM
        // for service-mode interop). The file manager and image viewer always run
        // as the same interactive user, so no other local account ever needs
        // access. Granting BUILTIN\Users (S-1-5-32-545) on a shared/RDP host
        // would otherwise let any other logged-on user inject crafted paths to
        // probe the viewer's parser or trigger UI actions in another session.
        let user_sid_bytes = current_process_user_sid()
            .map_err(|e| format!("failed to get current user SID: {}", e))?;
        let user_sid_len = user_sid_bytes.len();

        // NT AUTHORITY\SYSTEM SID: S-1-5-18
        let mut sid_system = [0u8; 12];
        sid_system[0] = 1;
        sid_system[1] = 1;
        sid_system[7] = 5;
        sid_system[8..12].copy_from_slice(&18u32.to_le_bytes());

        let ace1_size = 8 + user_sid_len;
        let ace2_size = 8 + sid_system.len();   // 20
        let acl_size = 8 + ace1_size + ace2_size;

        let mut acl_buffer = vec![0u8; acl_size];
        acl_buffer[0] = 2; // ACL_REVISION
        acl_buffer[2..4].copy_from_slice(&(acl_size as u16).to_le_bytes());
        acl_buffer[4..6].copy_from_slice(&2u16.to_le_bytes()); // AceCount

        // Current user: read+write access for pipe clients
        let access_mask_user: u32 = 0x0012019F;
        let access_mask_system: u32 = 0x001F01FF;

        let ace1_off = 8;
        acl_buffer[ace1_off] = 0; // ACCESS_ALLOWED_ACE_TYPE
        acl_buffer[ace1_off + 2..ace1_off + 4].copy_from_slice(&(ace1_size as u16).to_le_bytes());
        acl_buffer[ace1_off + 4..ace1_off + 8].copy_from_slice(&access_mask_user.to_le_bytes());
        acl_buffer[ace1_off + 8..ace1_off + 8 + user_sid_len].copy_from_slice(&user_sid_bytes);

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
    // SEC: Reject NUL bytes (can desync downstream Win32 string parsers) and
    // any non-UTF-8 sequence. The image viewer pipe carries only filesystem
    // paths; embedded NULs are always malicious here.
    if payload.contains(&0) {
        return Err("IPC payload contains NUL byte".to_string());
    }
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

/// SEC: Returns the raw bytes of the current process's user SID, suitable for
/// embedding directly into an ACE. Used to lock the image-viewer pipe DACL
/// down to the same user that owns the file-manager process.
fn current_process_user_sid() -> Result<Vec<u8>, String> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::{
        GetLengthSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token: HANDLE = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|e| format!("OpenProcessToken: {}", e))?;

        struct TokenGuard(HANDLE);
        impl Drop for TokenGuard {
            fn drop(&mut self) {
                unsafe {
                    let _ = CloseHandle(self.0);
                }
            }
        }
        let _guard = TokenGuard(token);

        // First call: get required size.
        let mut needed: u32 = 0;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        if needed == 0 {
            return Err("GetTokenInformation returned size 0".to_string());
        }

        let mut buffer = vec![0u8; needed as usize];
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr() as *mut _),
            needed,
            &mut needed,
        )
        .map_err(|e| format!("GetTokenInformation: {}", e))?;

        let token_user = &*(buffer.as_ptr() as *const TOKEN_USER);
        let sid_ptr = token_user.User.Sid;
        if sid_ptr.is_invalid() {
            return Err("token SID was null".to_string());
        }
        let sid_len = GetLengthSid(sid_ptr) as usize;
        if sid_len == 0 || sid_len > 256 {
            return Err(format!("unexpected SID length: {}", sid_len));
        }

        let mut sid_bytes = vec![0u8; sid_len];
        std::ptr::copy_nonoverlapping(sid_ptr.0 as *const u8, sid_bytes.as_mut_ptr(), sid_len);
        Ok(sid_bytes)
    }
}