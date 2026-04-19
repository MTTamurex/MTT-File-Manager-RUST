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

/// Timeout for the FolderSize request. NTFS folder sizes come from the
/// service's indexed subtree totals, but startup can still race with size
/// loading on the service side.
const FOLDER_SIZE_TIMEOUT_MS: u64 = 8_000;

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

/// Request the total size of a folder from the search service's in-memory
/// MFT-based index. Only works for NTFS volumes with sizes loaded.
/// Returns `(total_size, file_count, folder_count)` on success.
pub fn folder_size(path: &std::path::Path) -> Result<(u64, u64, u64), String> {
    let path_str = path.to_string_lossy().to_string();
    let pipe = open_pipe()?;

    let result = (|| {
        let request = SearchRequest::FolderSize { path: path_str };
        write_message(pipe, &request)?;
        let response = read_validated_response(pipe, FOLDER_SIZE_TIMEOUT_MS)?;

        match response {
            SearchResponse::FolderSize {
                total_size,
                file_count,
                folder_count,
                ..
            } => Ok((total_size, file_count, folder_count)),
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
/// Gets the server PID via `GetNamedPipeServerProcessId`, then validates BOTH:
///   (a) the running executable's basename is `mtt-search-service.exe`, and
///   (b) the process token's owning user is `NT AUTHORITY\SYSTEM`.
///
/// (b) is the strong guarantee: squatting the pipe under a LocalSystem token
/// already requires administrative privilege, at which point the local trust
/// boundary is gone anyway. (a) is a cheap sanity check to reject obvious
/// mistakes (e.g. another SYSTEM service that happens to win the pipe race).
///
/// We intentionally do NOT reject install locations based on substring
/// heuristics: the installer may legitimately place the service under
/// `C:\Program Files\...` or, during developer testing, under paths that
/// contain `\Users\` (e.g. a `target\release\` build tree). The token owner
/// check already covers the impersonation threat without false positives.
fn verify_server_process(pipe: HANDLE) -> Result<(), String> {
    use windows::Win32::Foundation::CloseHandle as Win32CloseHandle;
    use windows::Win32::Security::{
        GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
    };
    use windows::core::PWSTR;

    let mut server_pid: u32 = 0;
    unsafe {
        GetNamedPipeServerProcessId(pipe, &mut server_pid)
            .map_err(|e| format!("GetNamedPipeServerProcessId failed: {}", e))?;
    }

    if server_pid == 0 {
        return Err("Server PID is 0".into());
    }

    // Open the server process with the minimal access needed for image-name
    // and token queries. PROCESS_QUERY_LIMITED_INFORMATION works across
    // privilege levels (normal user can query a SYSTEM process for these).
    let process = unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, server_pid)
            .map_err(|e| format!("OpenProcess(server pid {}) failed: {}", server_pid, e))?
    };
    struct ProcGuard(HANDLE);
    impl Drop for ProcGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = Win32CloseHandle(self.0);
            }
        }
    }
    let _proc_guard = ProcGuard(process);

    // (a) Full image path lookup.
    let mut path_buf = [0u16; 1024];
    let mut path_len = path_buf.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_FORMAT(0),
            PWSTR(path_buf.as_mut_ptr()),
            &mut path_len,
        )
        .map_err(|e| format!("QueryFullProcessImageNameW failed: {}", e))?;
    }
    let exe_path = String::from_utf16_lossy(&path_buf[..path_len as usize]);
    let basename = std::path::Path::new(&exe_path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    if basename != "mtt-search-service.exe" {
        return Err(format!(
            "Server image basename is '{}', expected 'mtt-search-service.exe'",
            basename
        ));
    }

    // (b) Token owner must be NT AUTHORITY\SYSTEM (S-1-5-18).
    let mut token: HANDLE = HANDLE::default();
    unsafe {
        OpenProcessToken(process, TOKEN_QUERY, &mut token)
            .map_err(|e| format!("OpenProcessToken failed: {}", e))?;
    }
    struct TokenGuard(HANDLE);
    impl Drop for TokenGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = Win32CloseHandle(self.0);
            }
        }
    }
    let _tg = TokenGuard(token);

    let mut needed: u32 = 0;
    unsafe {
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
    }
    if needed == 0 || needed as usize > 4096 {
        return Err(format!("unexpected TokenUser size: {}", needed));
    }
    let mut buffer = vec![0u8; needed as usize];
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr() as *mut _),
            needed,
            &mut needed,
        )
        .map_err(|e| format!("GetTokenInformation failed: {}", e))?;
    }
    let token_user = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
    let sid_ptr = token_user.User.Sid;
    if sid_ptr.is_invalid() {
        return Err("Server token SID is null".into());
    }
    // Compare the raw SID bytes against the well-known LocalSystem SID.
    // S-1-5-18 = revision 1, 1 sub-authority, ID-auth 5, sub 18 → 12 bytes total.
    let expected: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0];
    let mut actual = [0u8; 12];
    unsafe {
        std::ptr::copy_nonoverlapping(sid_ptr.0 as *const u8, actual.as_mut_ptr(), 12);
    }
    if actual != expected {
        return Err(format!(
            "Server process is not running as LocalSystem (token SID mismatch)"
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
