use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, GetLastError, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_NONE, OPEN_EXISTING,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

pub const IMAGE_VIEWER_PIPE_NAME: &str = r"\\.\pipe\MTTFileManagerImageViewer";

const PIPE_BUFFER_SIZE: u32 = 32 * 1024;
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

    let pipe = unsafe {
        CreateNamedPipeW(
            PCWSTR(pipe_name_wide.as_ptr()),
            windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0x00000003),
            PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER_SIZE,
            PIPE_BUFFER_SIZE,
            0,
            None,
        )
    };

    if pipe.is_invalid() {
        return Err(format!("CreateNamedPipeW failed: {:?}", unsafe {
            GetLastError()
        }));
    }

    Ok(pipe)
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