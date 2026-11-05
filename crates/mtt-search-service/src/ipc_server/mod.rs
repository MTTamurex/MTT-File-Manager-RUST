mod connection_lifecycle;
mod connection_rejection;
mod folder_size_snapshot;
mod handler;
mod pipe_io;
mod volume_access;

use std::collections::HashMap;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::System::Pipes::ConnectNamedPipe;
use windows::Win32::System::Pipes::GetNamedPipeClientProcessId;
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::System::IO::OVERLAPPED;

use crate::indexing_progress::IndexingProgress;
use crate::security_policy::IpcSecurityPolicy;
use crate::volume_indices::SharedVolumeIndices;
use crate::volume_lifecycle::VolumeLifecycle;

const PIPE_BUFFER_SIZE: u32 = 64 * 1024;
const PIPE_MAX_INSTANCES: u32 = 32;
/// PIPE_ACCESS_DUPLEX (0x3) | FILE_FLAG_OVERLAPPED (0x40000000)
const PIPE_OPEN_MODE: u32 = 0x40000003;

/// Maximum concurrent client handler threads (rate limiting).
const MAX_ACTIVE_CLIENTS: u32 = 8;
/// Maximum concurrent handler threads from one client process.
const MAX_ACTIVE_CLIENTS_PER_PID: u32 = 4;
/// Maximum payload size for incoming requests (64 KB).
const MAX_REQUEST_PAYLOAD: usize = 64 * 1024;
/// Maximum results per query page.
const MAX_QUERY_RESULTS: usize = 10_000;
/// Maximum query offset to avoid pathological skip scans.
const MAX_QUERY_OFFSET: usize = 5_000_000;
/// Per-connection I/O timeout in seconds (prevents slowloris DoS).
const IO_TIMEOUT_SECS: u64 = 30;
const MAX_ACTIVE_REJECTIONS: u32 = 2;
const REJECTION_TIMEOUT_SECS: u64 = 2;

pub(super) enum WatchdogEvent {
    Extend(Duration),
    Done,
}

struct ClientSlot {
    active: Arc<AtomicU32>,
    by_pid: Arc<Mutex<HashMap<u32, u32>>>,
    pid: u32,
}

impl Drop for ClientSlot {
    fn drop(&mut self) {
        release_pid_slot(&self.by_pid, self.pid);
        self.active.fetch_sub(1, Ordering::Release);
    }
}

/// Start the IPC server loop.
pub fn run_ipc_server(
    indices: SharedVolumeIndices,
    indexing_progress: Arc<IndexingProgress>,
    shutdown: Arc<AtomicBool>,
    volume_lifecycle: Arc<VolumeLifecycle>,
) {
    let is_warming = Arc::new(AtomicBool::new(false));
    let last_warm_epoch_secs = Arc::new(AtomicU64::new(0));
    let active_clients = Arc::new(AtomicU32::new(0));
    let active_clients_by_pid = Arc::new(Mutex::new(HashMap::<u32, u32>::new()));
    let security_policy = Arc::new(IpcSecurityPolicy::from_env());
    let rejections = connection_rejection::RejectionPool::new(
        MAX_ACTIVE_REJECTIONS,
        Duration::from_secs(REJECTION_TIMEOUT_SECS),
    );

    if security_policy.redact_status_metrics {
        eprintln!("[IPC] Security policy: status metrics redaction is enabled");
    }

    // SEC: The first pipe instance uses FILE_FLAG_FIRST_PIPE_INSTANCE to detect
    // pre-emptive pipe squatting (another process created the pipe before us).
    let mut first_instance = true;

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let pipe = match pipe_io::create_pipe(first_instance) {
            Ok(p) => {
                first_instance = false;
                p
            }
            Err(e) => {
                if first_instance {
                    eprintln!(
                        "[IPC] SECURITY: Failed to create first pipe instance — \
                         another process may have squatted the pipe name: {}",
                        e
                    );
                } else {
                    eprintln!("[IPC] Failed to create pipe: {}", e);
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
                continue;
            }
        };
        let owned_pipe = unsafe { OwnedHandle::from_raw_handle(pipe.0) };

        // Wait for client with overlapped I/O so we can check shutdown periodically
        let client_connected = wait_for_client(pipe, &shutdown);
        if !client_connected {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            continue;
        }

        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let client_pid = match pipe_client_process_id(pipe) {
            Ok(pid) => pid,
            Err(error) => {
                eprintln!("[IPC] Failed to identify client process: {}", error);
                if let Err(error) = rejections.reject(
                    owned_pipe,
                    mtt_search_protocol::SearchResponse::Error("Authorization failed".to_string()),
                ) {
                    eprintln!("[IPC] Cannot schedule rejection: {error}");
                }
                continue;
            }
        };

        if !try_acquire_pid_slot(&active_clients_by_pid, client_pid) {
            eprintln!(
                "[IPC] Rate limit: rejecting pid {} (>{} active)",
                client_pid, MAX_ACTIVE_CLIENTS_PER_PID
            );
            if let Err(error) = rejections.reject(
                owned_pipe,
                mtt_search_protocol::SearchResponse::Error(
                    "Server busy, try again later".to_string(),
                ),
            ) {
                eprintln!("[IPC] Cannot schedule rejection: {error}");
            }
            continue;
        }

        // SEC: Atomic rate limiting -- fetch_add first, then rollback if over limit.
        // This closes the TOCTOU race where multiple threads could pass a load+compare
        // check simultaneously before any of them incremented the counter.
        let prev = active_clients.fetch_add(1, Ordering::AcqRel);
        if prev >= MAX_ACTIVE_CLIENTS {
            active_clients.fetch_sub(1, Ordering::Release);
            release_pid_slot(&active_clients_by_pid, client_pid);
            eprintln!(
                "[IPC] Rate limit: rejecting connection ({}/{} active)",
                prev, MAX_ACTIVE_CLIENTS
            );
            if let Err(error) = rejections.reject(
                owned_pipe,
                mtt_search_protocol::SearchResponse::Error(
                    "Server busy, try again later".to_string(),
                ),
            ) {
                eprintln!("[IPC] Cannot schedule rejection: {error}");
            }
            continue;
        }

        // Handle each client concurrently so one slow query doesn't block all connections.
        let indices_for_client = indices.clone();
        let progress_for_client = indexing_progress.clone();
        let warming_for_client = is_warming.clone();
        let warm_epoch_for_client = last_warm_epoch_secs.clone();
        let slot = ClientSlot {
            active: Arc::clone(&active_clients),
            by_pid: Arc::clone(&active_clients_by_pid),
            pid: client_pid,
        };
        let policy_for_client = security_policy.clone();
        let lifecycle_for_client = volume_lifecycle.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("search-ipc-client".into())
            .spawn(move || {
                let _slot = slot;
                if let Err(error) = connection_lifecycle::serve(
                    owned_pipe,
                    Duration::from_secs(IO_TIMEOUT_SECS),
                    |pipe, watchdog| {
                        handler::handle_client(
                            pipe,
                            &indices_for_client,
                            &progress_for_client,
                            handler::WarmIndexState {
                                is_warming: &warming_for_client,
                                last_warm_epoch_secs: &warm_epoch_for_client,
                            },
                            &policy_for_client,
                            &lifecycle_for_client,
                            watchdog,
                        )
                    },
                ) {
                    eprintln!("[IPC] Connection lifecycle startup failed: {error}");
                }
            })
        {
            // Dropping the unstarted closure releases the owned pipe and slot.
            eprintln!("[IPC] Cannot start client worker: {error}");
        }
    }

    // Graceful shutdown: wait up to 5 seconds for active client threads to finish.
    // Windows SCM has a 120s timeout, but we should not block that long.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let remaining = active_clients.load(Ordering::Acquire) + rejections.active_count();
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

fn watchdog_timed_out(events: &Receiver<WatchdogEvent>, timeout: Duration) -> bool {
    let started = std::time::Instant::now();
    let mut deadline = started + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match events.recv_timeout(remaining) {
            Ok(WatchdogEvent::Done) | Err(RecvTimeoutError::Disconnected) => return false,
            Ok(WatchdogEvent::Extend(total)) => {
                deadline = deadline.max(started + total);
            }
            Err(RecvTimeoutError::Timeout) => return true,
        }
    }
}

fn pipe_client_process_id(pipe: HANDLE) -> Result<u32, String> {
    let mut pid = 0u32;
    unsafe {
        GetNamedPipeClientProcessId(pipe, &mut pid)
            .map_err(|e| format!("GetNamedPipeClientProcessId failed: {}", e))?;
    }
    if pid == 0 {
        return Err("client pid is 0".to_string());
    }
    Ok(pid)
}

fn try_acquire_pid_slot(active_clients_by_pid: &Mutex<HashMap<u32, u32>>, pid: u32) -> bool {
    let mut counts = lock_pid_counts(active_clients_by_pid);
    let current = counts.get(&pid).copied().unwrap_or(0);
    if current >= MAX_ACTIVE_CLIENTS_PER_PID {
        return false;
    }
    counts.insert(pid, current + 1);
    true
}

fn release_pid_slot(active_clients_by_pid: &Mutex<HashMap<u32, u32>>, pid: u32) {
    let mut counts = lock_pid_counts(active_clients_by_pid);
    match counts.get_mut(&pid) {
        Some(count) if *count > 1 => *count -= 1,
        Some(_) => {
            counts.remove(&pid);
        }
        None => {}
    }
}

fn lock_pid_counts(
    active_clients_by_pid: &Mutex<HashMap<u32, u32>>,
) -> MutexGuard<'_, HashMap<u32, u32>> {
    active_clients_by_pid
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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

#[cfg(test)]
mod tests {
    use super::{watchdog_timed_out, WatchdogEvent};
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn client_slot_drop_releases_global_and_per_pid_capacity() {
        use super::{ClientSlot, HashMap};
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::{Arc, Mutex};

        let active = Arc::new(AtomicU32::new(2));
        let by_pid = Arc::new(Mutex::new(HashMap::from([(123, 2)])));
        let first = ClientSlot {
            active: Arc::clone(&active),
            by_pid: Arc::clone(&by_pid),
            pid: 123,
        };
        let second = ClientSlot {
            active: Arc::clone(&active),
            by_pid: Arc::clone(&by_pid),
            pid: 123,
        };
        drop(first);
        assert_eq!(active.load(Ordering::Acquire), 1);
        assert_eq!(by_pid.lock().unwrap().get(&123), Some(&1));
        drop(second);
        assert_eq!(active.load(Ordering::Acquire), 0);
        assert!(by_pid.lock().unwrap().is_empty());
    }

    #[test]
    fn watchdog_stops_when_handler_finishes() {
        let (done_tx, done_rx) = mpsc::channel();
        done_tx.send(WatchdogEvent::Done).unwrap();

        assert!(!watchdog_timed_out(&done_rx, Duration::from_millis(50)));
    }

    #[test]
    fn watchdog_reports_timeout_without_completion() {
        let (_done_tx, done_rx) = mpsc::channel();

        assert!(watchdog_timed_out(&done_rx, Duration::from_millis(5)));
    }

    #[test]
    fn watchdog_honors_operation_specific_extension() {
        let (tx, rx) = mpsc::channel();
        tx.send(WatchdogEvent::Extend(Duration::from_millis(100)))
            .unwrap();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            let _ = tx.send(WatchdogEvent::Done);
        });

        assert!(!watchdog_timed_out(&rx, Duration::from_millis(5)));
    }
}
