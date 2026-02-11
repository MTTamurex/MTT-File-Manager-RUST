use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Security::{
    InitializeSecurityDescriptor, SetSecurityDescriptorDacl, PSECURITY_DESCRIPTOR,
    SECURITY_ATTRIBUTES,
};
use windows::Win32::Storage::FileSystem::{FlushFileBuffers, ReadFile, WriteFile};
use windows::Win32::System::IO::OVERLAPPED;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_WAIT,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;

use crate::file_index::{self, IndexState, VolumeIndex};
use mtt_search_protocol::*;

const PIPE_BUFFER_SIZE: u32 = 64 * 1024;
/// PIPE_ACCESS_DUPLEX (0x3) | FILE_FLAG_OVERLAPPED (0x40000000)
const PIPE_OPEN_MODE: u32 = 0x40000003;

/// Start the IPC server loop.
pub fn run_ipc_server(indices: Arc<RwLock<Vec<VolumeIndex>>>, shutdown: Arc<AtomicBool>) {
    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let pipe = match create_pipe() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[IPC] Failed to create pipe: {}", e);
                std::thread::sleep(std::time::Duration::from_secs(1));
                continue;
            }
        };

        // Wait for client with overlapped I/O so we can check shutdown periodically
        let client_connected = match wait_for_client(pipe, &shutdown) {
            true => true,
            false => {
                unsafe {
                    let _ = CloseHandle(pipe);
                }
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }
                continue;
            }
        };

        if !client_connected || shutdown.load(Ordering::Relaxed) {
            unsafe {
                let _ = CloseHandle(pipe);
            }
            break;
        }

        handle_client(pipe, &indices);

        unsafe {
            let _ = FlushFileBuffers(pipe);
            let _ = DisconnectNamedPipe(pipe);
            let _ = CloseHandle(pipe);
        }
    }
}

/// Wait for a client connection using overlapped I/O with 1-second timeout polling.
/// Returns true if a client connected, false if shutdown or error.
fn wait_for_client(pipe: HANDLE, shutdown: &Arc<AtomicBool>) -> bool {
    unsafe {
        let event = match CreateEventW(None, true, false, None) {
            Ok(e) => e,
            Err(_) => return false,
        };

        let mut overlapped = OVERLAPPED::default();
        overlapped.hEvent = event;

        let result = ConnectNamedPipe(pipe, Some(&mut overlapped));

        if result.is_ok() {
            // Client already connected
            let _ = CloseHandle(event);
            return true;
        }

        let err = windows::Win32::Foundation::GetLastError();
        if err.0 == 535 {
            // ERROR_PIPE_CONNECTED: client connected before we called ConnectNamedPipe
            let _ = CloseHandle(event);
            return true;
        }
        if err.0 != 997 {
            // Not ERROR_IO_PENDING — real error
            eprintln!("[IPC] ConnectNamedPipe failed: error {}", err.0);
            let _ = CloseHandle(event);
            return false;
        }

        // ERROR_IO_PENDING: wait with timeout, checking shutdown flag
        loop {
            if shutdown.load(Ordering::Relaxed) {
                let _ = windows::Win32::System::IO::CancelIo(pipe);
                let _ = CloseHandle(event);
                return false;
            }

            let wait = WaitForSingleObject(event, 1000); // 1 second timeout
            if wait == WAIT_OBJECT_0 {
                let _ = CloseHandle(event);
                return true;
            }
            if wait == WAIT_TIMEOUT {
                // Timeout — loop back and check shutdown
                continue;
            }
            // WAIT_FAILED or other — error
            let _ = windows::Win32::System::IO::CancelIo(pipe);
            let _ = CloseHandle(event);
            return false;
        }
    }
}

fn create_pipe() -> Result<HANDLE, String> {
    unsafe {
        // Allocate a security descriptor buffer (SECURITY_DESCRIPTOR is opaque, needs ~40 bytes)
        let mut sd_buffer = vec![0u8; 256];
        let sd_ptr = PSECURITY_DESCRIPTOR(sd_buffer.as_mut_ptr() as *mut _);

        // SECURITY_DESCRIPTOR_REVISION = 1
        InitializeSecurityDescriptor(sd_ptr, 1)
            .map_err(|e| format!("InitializeSecurityDescriptor: {}", e))?;

        // NULL DACL = allow all access (so non-admin app can connect)
        SetSecurityDescriptorDacl(sd_ptr, true, None, false)
            .map_err(|e| format!("SetSecurityDescriptorDacl: {}", e))?;

        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd_ptr.0,
            bInheritHandle: false.into(),
        };

        let pipe_name: Vec<u16> = PIPE_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let pipe = CreateNamedPipeW(
            PCWSTR(pipe_name.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(PIPE_OPEN_MODE),
            PIPE_WAIT, // BYTE mode — our protocol already does length-prefix framing
            1,
            PIPE_BUFFER_SIZE,
            PIPE_BUFFER_SIZE,
            0,
            Some(&sa as *const _),
        );

        if pipe.is_invalid() {
            return Err(format!(
                "CreateNamedPipeW failed: {:?}",
                windows::Win32::Foundation::GetLastError()
            ));
        }

        Ok(pipe)
    }
}

fn handle_client(pipe: HANDLE, indices: &Arc<RwLock<Vec<VolumeIndex>>>) {
    let request_data = match read_message(pipe) {
        Some(data) => data,
        None => return,
    };

    let request: SearchRequest = match decode_message(&request_data) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[IPC] Failed to decode request: {}", e);
            let _ = send_response(pipe, &SearchResponse::Error(e));
            return;
        }
    };

    match request {
        SearchRequest::Ping => {
            let _ = send_response(pipe, &SearchResponse::Pong);
        }
        SearchRequest::GetStatus => {
            let indices_lock = indices.read().unwrap();
            let mut total_indexed = 0u64;
            let mut volumes = Vec::new();

            for vol in indices_lock.iter() {
                let count = vol.records.len() as u64;
                total_indexed += count;
                volumes.push(VolumeStatus {
                    drive_letter: vol.drive_letter,
                    state: match &vol.state {
                        IndexState::NotStarted => "not_started".to_string(),
                        IndexState::Scanning => "scanning".to_string(),
                        IndexState::Ready => "ready".to_string(),
                        IndexState::Error(e) => format!("error: {}", e),
                    },
                    files_indexed: count,
                });
            }

            let _ = send_response(
                pipe,
                &SearchResponse::Status(IndexStatusInfo {
                    volumes,
                    total_files_indexed: total_indexed,
                }),
            );
        }
        SearchRequest::Query { text, max_results } => {
            let indices_lock = indices.read().unwrap();
            let results = file_index::search(&indices_lock, &text, max_results as usize);

            let items: Vec<SearchResultItem> = results
                .into_iter()
                .map(|r| SearchResultItem {
                    name: r.name,
                    full_path: r.full_path,
                    is_dir: r.is_dir,
                    size: r.size,
                })
                .collect();

            let total = items.len() as u32;
            let _ = send_response(
                pipe,
                &SearchResponse::Results {
                    items,
                    is_final: true,
                    total_found: total,
                },
            );
        }
    }
}

fn read_message(pipe: HANDLE) -> Option<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    let mut bytes_read: u32 = 0;

    let ok = unsafe { ReadFile(pipe, Some(&mut len_buf), Some(&mut bytes_read), None) };

    if ok.is_err() || bytes_read != 4 {
        return None;
    }

    let payload_len = u32::from_le_bytes(len_buf) as usize;
    if payload_len == 0 || payload_len > 10 * 1024 * 1024 {
        return None;
    }

    let mut payload = vec![0u8; payload_len];
    let mut total_read = 0usize;

    while total_read < payload_len {
        let mut chunk_read: u32 = 0;
        let ok = unsafe {
            ReadFile(
                pipe,
                Some(&mut payload[total_read..]),
                Some(&mut chunk_read),
                None,
            )
        };

        if ok.is_err() || chunk_read == 0 {
            return None;
        }

        total_read += chunk_read as usize;
    }

    Some(payload)
}

fn send_response(pipe: HANDLE, response: &SearchResponse) -> Result<(), String> {
    let encoded =
        encode_message(response).map_err(|e| format!("Failed to encode response: {}", e))?;

    let mut bytes_written: u32 = 0;
    unsafe {
        WriteFile(pipe, Some(&encoded), Some(&mut bytes_written), None)
            .map_err(|e| format!("WriteFile failed: {}", e))?;
    }

    Ok(())
}
