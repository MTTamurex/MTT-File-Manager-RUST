//! Client for communicating with the MTT Search Service via Named Pipes.

use mtt_search_protocol::*;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_BUSY, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_NONE, OPEN_EXISTING,
};
use windows::Win32::System::Pipes::PeekNamedPipe;

const PIPE_IO_TIMEOUT_MS: u64 = 8000;
const PIPE_POLL_INTERVAL_MS: u64 = 15;

/// Send a search query to the service and return results.
pub fn search(query: &str, max_results: u32) -> Result<Vec<SearchResultItem>, String> {
    let pipe = open_pipe()?;

    let result = (|| {
        let request = SearchRequest::Query {
            text: query.to_string(),
            max_results,
        };
        write_message(pipe, &request)?;
        let response: SearchResponse = read_response(pipe)?;

        match response {
            SearchResponse::Results { items, .. } => Ok(items),
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
        let response: SearchResponse = read_response(pipe)?;

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
    const ATTEMPTS: usize = 3;
    for attempt in 0..ATTEMPTS {
        let pipe = match open_pipe() {
            Ok(pipe) => pipe,
            Err(e) => {
                // Service may be saturated but alive; don't mark as offline immediately.
                if e.contains("All pipe instances are busy") {
                    eprintln!("[GLOBAL-SEARCH] Ping: service busy");
                    return true;
                }
                return false;
            }
        };

        let ping_write = write_message(pipe, &SearchRequest::Ping);
        let ping_read = if ping_write.is_ok() {
            read_response::<SearchResponse>(pipe)
        } else {
            Err(ping_write
                .err()
                .unwrap_or_else(|| "Ping write failed".to_string()))
        };

        unsafe {
            let _ = CloseHandle(pipe);
        }

        if matches!(ping_read, Ok(SearchResponse::Pong)) {
            return true;
        }

        let transient = match &ping_read {
            Ok(_) => false,
            Err(e) => is_transient_pipe_error(e),
        };
        if transient && attempt + 1 < ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(120));
            continue;
        }

        if transient {
            // Keep optimistic online signal for transient pipe races.
            return true;
        }

        if let Err(e) = ping_read {
            eprintln!("[GLOBAL-SEARCH] Ping failed: {}", e);
        }
        return false;
    }

    false
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
        let response: SearchResponse = read_response(pipe)?;

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
                Ok(handle) => return Ok(handle),
                Err(e) => {
                    let code = e.code();
                    if code == ERROR_PIPE_BUSY.to_hresult() {
                        // Service is alive but all pipe instances are busy — worth retrying.
                        last_error =
                            "All pipe instances are busy".to_string();
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

fn write_message<T: serde::Serialize>(pipe: HANDLE, msg: &T) -> Result<(), String> {
    let encoded = encode_message(msg)?;

    let mut bytes_written: u32 = 0;
    unsafe {
        WriteFile(pipe, Some(&encoded), Some(&mut bytes_written), None)
            .map_err(|e| format!("WriteFile failed: {}", e))?;
    }

    Ok(())
}

fn read_response<T: for<'de> serde::Deserialize<'de>>(pipe: HANDLE) -> Result<T, String> {
    // Read 4-byte length prefix
    let mut len_buf = [0u8; 4];
    read_exact_with_timeout(pipe, &mut len_buf, PIPE_IO_TIMEOUT_MS)?;

    let payload_len = u32::from_le_bytes(len_buf) as usize;
    if payload_len == 0 || payload_len > 10 * 1024 * 1024 {
        return Err(format!("Invalid payload length: {}", payload_len));
    }

    // Read payload
    let mut payload = vec![0u8; payload_len];
    read_exact_with_timeout(pipe, &mut payload, PIPE_IO_TIMEOUT_MS)?;

    decode_message(&payload)
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
