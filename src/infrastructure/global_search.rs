//! Client for communicating with the MTT Search Service via Named Pipes.

use mtt_search_protocol::*;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_BUSY, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_NONE, OPEN_EXISTING,
};
use windows::Win32::System::Pipes::{GetNamedPipeServerProcessId, PeekNamedPipe};

const SEARCH_PIPE_IO_TIMEOUT_MS: u64 = 8_000;
const CONTROL_PIPE_IO_TIMEOUT_MS: u64 = 5_000;
const PIPE_POLL_INTERVAL_MS: u64 = 15;

pub struct SearchPage {
    pub items: Vec<SearchResultItem>,
    pub has_more: bool,
    pub total_matches: Option<u32>,
}

/// Send a search query to the service and return results.
pub fn search(query: &str, offset: u32, limit: u32) -> Result<SearchPage, String> {
    let pipe = open_pipe()?;

    let result = (|| {
        let request = SearchRequest::Query {
            text: query.to_string(),
            offset,
            limit,
        };
        write_message(pipe, &request)?;
        let response = read_validated_response(pipe, SEARCH_PIPE_IO_TIMEOUT_MS)?;

        match response {
            SearchResponse::Results {
                items,
                has_more,
                total_matches,
            } => Ok(SearchPage {
                items,
                has_more,
                total_matches,
            }),
            SearchResponse::Error(e) => Err(e),
            _ => Err("Unexpected response type".into()),
        }
    })();

    unsafe {
        let _ = CloseHandle(pipe);
    }

    result
}

/// Ask the service to warm its in-memory index, bringing paged-out memory back to RAM.
/// Fire-and-forget: the service responds immediately and warms in the background.
pub fn warm_index() -> Result<(), String> {
    let pipe = open_pipe()?;

    let result = (|| {
        write_message(pipe, &SearchRequest::WarmIndex)?;
        let response = read_validated_response(pipe, CONTROL_PIPE_IO_TIMEOUT_MS)?;

        match response {
            SearchResponse::WarmStarted => Ok(()),
            SearchResponse::Error(e) => Err(e),
            _ => Err("Unexpected response type".into()),
        }
    })();

    unsafe {
        let _ = CloseHandle(pipe);
    }

    result
}

/// Check if the service is running.
pub fn ping() -> bool {
    let pipe = match open_pipe() {
        Ok(pipe) => pipe,
        Err(e) => {
            // Service may be saturated but alive; don't mark as offline immediately.
            if e.contains("All pipe instances are busy") {
                log::debug!("[GLOBAL-SEARCH] Ping: service busy");
                return true;
            }
            return false;
        }
    };

    let ping_write = write_message(pipe, &SearchRequest::Ping);
    let ping_read = if ping_write.is_ok() {
        read_validated_response(pipe, CONTROL_PIPE_IO_TIMEOUT_MS)
    } else {
        Err(ping_write
            .err()
            .unwrap_or_else(|| "Ping write failed".to_string()))
    };

    unsafe {
        let _ = CloseHandle(pipe);
    }

    match &ping_read {
        Ok(SearchResponse::Pong) => true,
        Ok(_) => false,
        Err(e) => {
            // Transient pipe errors still mean the service is alive.
            if is_transient_pipe_error(e) {
                return true;
            }
            log::warn!("[GLOBAL-SEARCH] Ping failed: {}", e);
            false
        }
    }
}

fn is_transient_pipe_error(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("all pipe instances are busy")
        || m.contains("no process is on the other end of the pipe")
        || m.contains("pipe closed during read")
        || m.contains("peeknamedpipe failed")
        || m.contains("search service timeout")
        || m.contains("readfile failed")
        || m.contains("writefile failed")
}

/// Get index status from the service.
pub fn get_status() -> Result<IndexStatusInfo, String> {
    let pipe = open_pipe()?;

    let result = (|| {
        write_message(pipe, &SearchRequest::GetStatus)?;
        let response = read_validated_response(pipe, CONTROL_PIPE_IO_TIMEOUT_MS)?;

        match response {
            SearchResponse::Status(info) => Ok(info),
            SearchResponse::Error(e) => Err(e),
            _ => Err("Unexpected response type".into()),
        }
    })();

    unsafe {
        let _ = CloseHandle(pipe);
    }

    result
}

/// Timeout for the lightweight CheckPathsModified request.
const CHECK_PATHS_TIMEOUT_MS: u64 = 2_000;

/// Ask the search service which of the given directory paths have been
/// modified (via USN journal) within the last `threshold_secs` seconds.
/// Returns the subset of paths that changed. Useful for tab-switch staleness
/// detection on NTFS volumes without any disk I/O in the app process.
pub fn check_paths_modified(paths: &[String], threshold_secs: u32) -> Result<Vec<String>, String> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }

    let pipe = open_pipe()?;

    let result = (|| {
        let request = SearchRequest::CheckPathsModified {
            paths: paths.to_vec(),
            threshold_secs,
        };
        write_message(pipe, &request)?;
        let response = read_validated_response(pipe, CHECK_PATHS_TIMEOUT_MS)?;

        match response {
            SearchResponse::PathsModified { modified } => Ok(modified),
            SearchResponse::Error(e) => Err(e),
            _ => Err("Unexpected response type".into()),
        }
    })();

    unsafe {
        let _ = CloseHandle(pipe);
    }

    result
}

fn open_pipe() -> Result<HANDLE, String> {
    let pipe_name_wide: Vec<u16> = PIPE_NAME.encode_utf16().chain(std::iter::once(0)).collect();

    // Only retry on PIPE_BUSY (service alive but all instances occupied).
    // FILE_NOT_FOUND means the service isn't running — fail immediately
    // instead of blocking the worker thread for seconds.
    const BUSY_RETRY_COUNT: usize = 6;
    const BUSY_WAIT_MS: u64 = 150;

    let mut last_error = String::from("Search service not available");
    for _ in 0..BUSY_RETRY_COUNT {
        unsafe {
            match CreateFileW(
                PCWSTR(pipe_name_wide.as_ptr()),
                0x80000000 | 0x40000000, // GENERIC_READ | GENERIC_WRITE
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            ) {
                Ok(handle) => {
                    // SEC: Verify the server process is the legitimate search service.
                    // A rogue process could squat the pipe name and impersonate the service.
                    if let Err(e) = verify_server_process(handle) {
                        log::warn!("[SEARCH-CLIENT] Pipe server verification failed: {}", e);
                        let _ = CloseHandle(handle);
                        return Err(format!("Pipe server verification failed: {}", e));
                    }
                    return Ok(handle);
                }
                Err(e) => {
                    let code = e.code();
                    if code == ERROR_PIPE_BUSY.to_hresult() {
                        // Service is alive but all pipe instances are busy — worth retrying.
                        last_error = "All pipe instances are busy".to_string();
                        std::thread::sleep(std::time::Duration::from_millis(BUSY_WAIT_MS));
                        continue;
                    }
                    // FILE_NOT_FOUND or any other error — service not running, fail fast.
                    return Err(format!("Search service not available: {}", e));
                }
            }
        }
    }

    Err(last_error)
}

/// SEC: Verify that the named pipe server belongs to the legitimate search service.
/// Gets the server PID and checks that its executable is `mtt-search-service.exe`.
fn verify_server_process(pipe: HANDLE) -> Result<(), String> {
    let mut server_pid: u32 = 0;
    unsafe {
        GetNamedPipeServerProcessId(pipe, &mut server_pid)
            .map_err(|e| format!("GetNamedPipeServerProcessId failed: {}", e))?;
    }

    if server_pid == 0 {
        return Err("Server PID is 0".into());
    }

    // Open the process and query its image name.
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    let process = unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, server_pid)
            .map_err(|e| format!("OpenProcess({}) failed: {}", server_pid, e))?
    };

    let mut buf = [0u16; 512];
    let mut len = buf.len() as u32;
    let result = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    };
    unsafe {
        let _ = CloseHandle(process);
    }
    result.map_err(|e| format!("QueryFullProcessImageNameW failed: {}", e))?;

    let image_path = String::from_utf16_lossy(&buf[..len as usize]);
    let file_name = image_path
        .rsplit('\\')
        .next()
        .unwrap_or("")
        .to_lowercase();

    if file_name != "mtt-search-service.exe" {
        return Err(format!(
            "Server process is '{}', expected 'mtt-search-service.exe'",
            file_name
        ));
    }

    Ok(())
}

fn write_message<T: serde::Serialize>(pipe: HANDLE, msg: &T) -> Result<(), String> {
    let encoded = encode_message(msg)?;
    write_all(pipe, &encoded)
}

fn write_all(pipe: HANDLE, data: &[u8]) -> Result<(), String> {
    let mut offset = 0usize;

    while offset < data.len() {
        let mut bytes_written: u32 = 0;
        unsafe {
            WriteFile(pipe, Some(&data[offset..]), Some(&mut bytes_written), None)
                .map_err(|e| format!("WriteFile failed: {}", e))?;
        }

        if bytes_written == 0 {
            return Err("Pipe closed during write".into());
        }

        let written = bytes_written as usize;
        if written > data.len().saturating_sub(offset) {
            return Err("WriteFile wrote beyond buffer bounds".into());
        }

        offset += written;
    }

    Ok(())
}

fn read_response<T: for<'de> serde::Deserialize<'de>>(
    pipe: HANDLE,
    timeout_ms: u64,
) -> Result<T, String> {
    // Read 4-byte length prefix
    let mut len_buf = [0u8; 4];
    read_exact_with_timeout(pipe, &mut len_buf, timeout_ms)?;

    let payload_len = u32::from_le_bytes(len_buf) as usize;
    if payload_len == 0 || payload_len > 1024 * 1024 {
        return Err(format!("Invalid payload length: {}", payload_len));
    }

    // Read payload
    let mut payload = vec![0u8; payload_len];
    read_exact_with_timeout(pipe, &mut payload, timeout_ms)?;

    decode_message(&payload)
}

/// Reads and validates a [`SearchResponse`] from the pipe. Returns an error if
/// the response fails post-deserialization validation (e.g. too many items).
fn read_validated_response(
    pipe: HANDLE,
    timeout_ms: u64,
) -> Result<SearchResponse, String> {
    let resp: SearchResponse = read_response(pipe, timeout_ms)?;
    resp.validate()?;
    Ok(resp)
}

fn read_exact_with_timeout(pipe: HANDLE, buf: &mut [u8], timeout_ms: u64) -> Result<(), String> {
    let start = std::time::Instant::now();
    let mut offset = 0usize;

    while offset < buf.len() {
        if start.elapsed() > std::time::Duration::from_millis(timeout_ms) {
            return Err(format!(
                "Search service timeout waiting response ({}ms)",
                timeout_ms
            ));
        }

        let mut total_avail: u32 = 0;
        unsafe {
            PeekNamedPipe(pipe, None, 0, None, Some(&mut total_avail), None)
                .map_err(|e| format!("PeekNamedPipe failed: {}", e))?;
        }

        if total_avail == 0 {
            std::thread::sleep(std::time::Duration::from_millis(PIPE_POLL_INTERVAL_MS));
            continue;
        }

        let remaining = buf.len() - offset;
        let to_read = remaining.min(total_avail as usize);
        let mut bytes_read: u32 = 0;
        unsafe {
            ReadFile(
                pipe,
                Some(&mut buf[offset..offset + to_read]),
                Some(&mut bytes_read),
                None,
            )
            .map_err(|e| format!("ReadFile failed: {}", e))?;
        }

        if bytes_read == 0 {
            return Err("Pipe closed during read".into());
        }

        offset += bytes_read as usize;
    }

    Ok(())
}
