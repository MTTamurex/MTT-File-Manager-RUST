//! Client for communicating with the MTT Search Service via Named Pipes.

use mtt_search_protocol::*;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_NONE, OPEN_EXISTING,
};

/// Send a search query to the service and return results.
pub fn search(query: &str, max_results: u32) -> Result<Vec<SearchResultItem>, String> {
    let pipe = open_pipe()?;

    let request = SearchRequest::Query {
        text: query.to_string(),
        max_results,
    };
    write_message(pipe, &request)?;
    let response: SearchResponse = read_response(pipe)?;

    unsafe {
        let _ = CloseHandle(pipe);
    }

    match response {
        SearchResponse::Results { items, .. } => Ok(items),
        SearchResponse::Error(e) => Err(e),
        _ => Err("Unexpected response type".into()),
    }
}

/// Check if the service is running.
pub fn ping() -> bool {
    let Ok(pipe) = open_pipe() else {
        return false;
    };

    let ok = write_message(pipe, &SearchRequest::Ping).is_ok()
        && matches!(
            read_response::<SearchResponse>(pipe),
            Ok(SearchResponse::Pong)
        );

    unsafe {
        let _ = CloseHandle(pipe);
    }

    ok
}

/// Get index status from the service.
pub fn get_status() -> Result<IndexStatusInfo, String> {
    let pipe = open_pipe()?;

    write_message(pipe, &SearchRequest::GetStatus)?;
    let response: SearchResponse = read_response(pipe)?;

    unsafe {
        let _ = CloseHandle(pipe);
    }

    match response {
        SearchResponse::Status(info) => Ok(info),
        SearchResponse::Error(e) => Err(e),
        _ => Err("Unexpected response type".into()),
    }
}

fn open_pipe() -> Result<HANDLE, String> {
    let pipe_name_wide: Vec<u16> = PIPE_NAME
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let handle = CreateFileW(
            PCWSTR(pipe_name_wide.as_ptr()),
            0x80000000 | 0x40000000, // GENERIC_READ | GENERIC_WRITE
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .map_err(|e| format!("Search service not available: {}", e))?;

        Ok(handle)
    }
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
    let mut bytes_read: u32 = 0;

    unsafe {
        ReadFile(pipe, Some(&mut len_buf), Some(&mut bytes_read), None)
            .map_err(|e| format!("ReadFile (length) failed: {}", e))?;
    }

    if bytes_read != 4 {
        return Err("Incomplete length prefix".into());
    }

    let payload_len = u32::from_le_bytes(len_buf) as usize;
    if payload_len == 0 || payload_len > 10 * 1024 * 1024 {
        return Err(format!("Invalid payload length: {}", payload_len));
    }

    // Read payload
    let mut payload = vec![0u8; payload_len];
    let mut total_read = 0usize;

    while total_read < payload_len {
        let mut chunk_read: u32 = 0;
        unsafe {
            ReadFile(
                pipe,
                Some(&mut payload[total_read..]),
                Some(&mut chunk_read),
                None,
            )
            .map_err(|e| format!("ReadFile (payload) failed: {}", e))?;
        }

        if chunk_read == 0 {
            return Err("Pipe closed during read".into());
        }

        total_read += chunk_read as usize;
    }

    decode_message(&payload)
}
