mod handler;
mod pipe_io;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use parking_lot::RwLock;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Storage::FileSystem::FlushFileBuffers;
use windows::Win32::System::Pipes::{ConnectNamedPipe, DisconnectNamedPipe};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::System::IO::OVERLAPPED;

use crate::file_index::VolumeIndex;
use crate::index_db;
use crate::security_policy::IpcSecurityPolicy;

const PIPE_BUFFER_SIZE: u32 = 64 * 1024;
const PIPE_MAX_INSTANCES: u32 = 32;
/// PIPE_ACCESS_DUPLEX (0x3) | FILE_FLAG_OVERLAPPED (0x40000000)
const PIPE_OPEN_MODE: u32 = 0x40000003;

/// Maximum concurrent client handler threads (rate limiting).
const MAX_ACTIVE_CLIENTS: u32 = 8;
/// Maximum payload size for incoming requests (64 KB).
const MAX_REQUEST_PAYLOAD: usize = 64 * 1024;
/// Maximum search query text length in bytes.
const MAX_QUERY_TEXT_LEN: usize = 1024;
/// Maximum results per query page.
const MAX_QUERY_RESULTS: usize = 10_000;
/// Maximum query offset to avoid pathological skip scans.
const MAX_QUERY_OFFSET: usize = 5_000_000;
/// Per-connection I/O timeout in seconds (prevents slowloris DoS).
const IO_TIMEOUT_SECS: u64 = 30;

/// Start the IPC server loop.
pub fn run_ipc_server(
    indices: Arc<RwLock<Vec<VolumeIndex>>>,
    shutdown: Arc<AtomicBool>,
    db_path: std::path::PathBuf,
) {
    let is_warming = Arc::new(AtomicBool::new(false));
    let active_clients = Arc::new(AtomicU32::new(0));
    let security_policy = Arc::new(IpcSecurityPolicy::from_env());

    // FTS5 searcher: separate read-only SQLite connection for fast queries.
    let fts_searcher: Option<Arc<index_db::FtsSearcher>> =
        match index_db::FtsSearcher::open(&db_path) {
            Ok(s) => {
                eprintln!("[IPC] FTS5 searcher ready");
                Some(Arc::new(s))
            }
            Err(e) => {
                eprintln!("[IPC] FTS5 searcher unavailable, falling back to linear scan: {}", e);
                None
            }
        };

    if security_policy.redact_status_metrics {
        eprintln!("[IPC] Security policy: status metrics redaction is enabled");
    }

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let pipe = match pipe_io::create_pipe() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[IPC] Failed to create pipe: {}", e);
                std::thread::sleep(std::time::Duration::from_secs(1));
                continue;
            }
        };

        // Wait for client with overlapped I/O so we can check shutdown periodically
        let client_connected = wait_for_client(pipe, &shutdown);
        if !client_connected {
            unsafe {
                let _ = CloseHandle(pipe);
            }
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            continue;
        }

        if shutdown.load(Ordering::Relaxed) {
            unsafe {
                let _ = CloseHandle(pipe);
            }
            break;
        }

        // Rate limiting: reject if too many concurrent clients
        let current = active_clients.load(Ordering::Relaxed);
        if current >= MAX_ACTIVE_CLIENTS {
            eprintln!(
                "[IPC] Rate limit: rejecting connection ({}/{} active)",
                current, MAX_ACTIVE_CLIENTS
            );
            // Try to send an error response before disconnecting
            let _ = pipe_io::send_response(
                pipe,
                &mtt_search_protocol::SearchResponse::Error("Server busy, try again later".to_string()),
            );
            unsafe {
                let _ = FlushFileBuffers(pipe);
                let _ = DisconnectNamedPipe(pipe);
                let _ = CloseHandle(pipe);
            }
            continue;
        }

        active_clients.fetch_add(1, Ordering::Relaxed);

        // Handle each client concurrently so one slow query doesn't block all connections.
        let indices_for_client = indices.clone();
        let warming_for_client = is_warming.clone();
        let active_for_client = active_clients.clone();
        let policy_for_client = security_policy.clone();
        let fts_for_client = fts_searcher.clone();
        let pipe_raw = pipe.0 as usize;
        std::thread::spawn(move || {
            let pipe = HANDLE(pipe_raw as *mut core::ffi::c_void);

            // Watchdog thread: disconnects the pipe if the client exceeds
            // IO_TIMEOUT_SECS, preventing slowloris-style DoS that would
            // exhaust the MAX_ACTIVE_CLIENTS handler pool.
            let client_done = Arc::new(AtomicBool::new(false));
            let watchdog_done = client_done.clone();
            let watchdog_pipe = pipe_raw;
            std::thread::spawn(move || {
                for _ in 0..IO_TIMEOUT_SECS {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    if watchdog_done.load(Ordering::Relaxed) {
                        return;
                    }
                }
                if !watchdog_done.load(Ordering::Relaxed) {
                    eprintln!(
                        "[IPC] Client timeout after {}s, disconnecting",
                        IO_TIMEOUT_SECS
                    );
                    unsafe {
                        let handle = HANDLE(watchdog_pipe as *mut core::ffi::c_void);
                        let _ = DisconnectNamedPipe(handle);
                    }
                }
            });

            if let Err(e) = catch_unwind(AssertUnwindSafe(|| {
                handler::handle_client(pipe, &indices_for_client, &warming_for_client, &policy_for_client, &fts_for_client)
            })) {
                eprintln!("[IPC] Client handler panic: {:?}", e);
            }
            client_done.store(true, Ordering::Relaxed);
            unsafe {
                let _ = FlushFileBuffers(pipe);
                let _ = DisconnectNamedPipe(pipe);
                let _ = CloseHandle(pipe);
            }
            active_for_client.fetch_sub(1, Ordering::Relaxed);
        });
    }

    // Graceful shutdown: wait up to 5 seconds for active client threads to finish.
    // Windows SCM has a 120s timeout, but we should not block that long.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let remaining = active_clients.load(Ordering::Relaxed);
        if remaining == 0 {
            break;
        }
        if std::time::Instant::now() >= deadline {
            eprintln!(
                "[IPC] Shutdown timeout: {} client(s) still active, force-exiting",
                remaining
            );
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
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

        let mut overlapped = OVERLAPPED {
            hEvent: event,
            ..Default::default()
        };

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
