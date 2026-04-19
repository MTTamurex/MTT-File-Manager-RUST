use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    ReadDirectoryChangesW, FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_CREATION,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
    FILE_NOTIFY_CHANGE_SIZE,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};

use crate::infrastructure::windows::OwnedHandle;

use super::buffer_parser::{event_matches_prefix, parse_notify_buffer};
use super::{DriveWatcherEvent, WatcherCommand};

/// Buffer size for directory change notifications.
/// We watch local drive roots, so a larger buffer is safe and reduces overflow risk.
const BUFFER_SIZE: usize = 256 * 1024;

/// Wrapper to ensure the I/O buffer has alignment required by
/// `FILE_NOTIFY_INFORMATION` (4 bytes).  A plain `Vec<u8>` has alignment=1.
#[repr(C, align(8))]
struct AlignedBuffer([u8; BUFFER_SIZE]);

/// Maximum events to keep in the dedup buffer before flushing.
/// When exceeded, events are coalesced into a bulk invalidation.
const MAX_COALESCED_EVENTS: usize = 500;

/// Minimum interval (ms) between sending event batches to the UI thread.
/// This prevents flooding the channel during OneDrive dehydration storms.
const COALESCE_INTERVAL_MS: u64 = 200;

/// Main watcher thread function.
///
/// ARCHITECTURE (inspired by Files app):
/// - Events are coalesced in a HashSet to deduplicate (same path -> one event)
/// - Batches are flushed at most every COALESCE_INTERVAL_MS (200ms)
/// - When the buffer exceeds MAX_COALESCED_EVENTS, it's flushed immediately
///   to prevent unbounded memory growth during OneDrive dehydration storms
/// - This ensures the UI thread never receives unbounded event lists
pub(super) fn watcher_thread_main(
    handle: HANDLE,
    drive_root: PathBuf,
    command_rx: std::sync::mpsc::Receiver<WatcherCommand>,
    event_tx: std::sync::mpsc::Sender<Vec<DriveWatcherEvent>>,
    shutdown: Arc<AtomicBool>,
    mut current_prefix: PathBuf,
) {
    log::info!("[DRIVE-WATCHER] Thread started for drive: {:?}", drive_root);

    unsafe {
        let Some(handle) = OwnedHandle::new(handle) else {
            log::error!("[DRIVE-WATCHER] Received invalid drive handle for {:?}", drive_root);
            return;
        };

        // Create events for overlapped I/O.
        let h_event = match CreateEventW(None, true, false, None) {
            Ok(event) => match OwnedHandle::new(event) {
                Some(handle) => handle,
                None => {
                    log::error!("[DRIVE-WATCHER] CreateEventW returned invalid handle");
                    return;
                }
            },
            Err(e) => {
                log::error!("[DRIVE-WATCHER] Failed to create event: {}", e);
                return;
            }
        };

        // Buffer for directory change notifications.
        let mut buffer = AlignedBuffer([0u8; BUFFER_SIZE]);
        let mut overlapped = std::mem::zeroed::<OVERLAPPED>();
        overlapped.hEvent = h_event.as_raw();

        // Coalescing state: events are deduplicated here before sending.
        let mut coalesced: HashSet<DriveWatcherEvent> = HashSet::new();
        let mut last_flush = std::time::Instant::now();
        let mut bytes_returned: u32 = 0;
        let mut waiting_for_io = false;

        loop {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }

            // Drain commands (non-blocking), keeping the latest prefix.
            let mut should_exit = false;
            loop {
                match command_rx.try_recv() {
                    Ok(WatcherCommand::UpdatePrefix(new_prefix)) => {
                        current_prefix = new_prefix;
                    }
                    Ok(WatcherCommand::Shutdown) => {
                        should_exit = true;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        should_exit = true;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                }
            }
            if should_exit {
                break;
            }

            // Start async read if not already pending.
            if !waiting_for_io {
                buffer.0.fill(0);
                bytes_returned = 0;
                let h_event = overlapped.hEvent;
                overlapped = std::mem::zeroed::<OVERLAPPED>();
                overlapped.hEvent = h_event;
                let result = ReadDirectoryChangesW(
                    handle.as_raw(),
                    buffer.0.as_mut_ptr() as *mut _,
                    buffer.0.len() as u32,
                    true, // Watch subtree (entire drive)
                    FILE_NOTIFY_CHANGE_FILE_NAME
                        | FILE_NOTIFY_CHANGE_DIR_NAME
                        | FILE_NOTIFY_CHANGE_ATTRIBUTES
                        | FILE_NOTIFY_CHANGE_SIZE
                        | FILE_NOTIFY_CHANGE_LAST_WRITE
                        | FILE_NOTIFY_CHANGE_CREATION,
                    None, // Async operation - bytes returned comes from GetOverlappedResult
                    Some(&mut overlapped),
                    None,
                );

                if result.is_err() {
                    log::error!(
                        "[DRIVE-WATCHER] ReadDirectoryChangesW failed (drive likely unmounted): {:?}",
                        result.err()
                    );
                    let _ = event_tx.send(vec![DriveWatcherEvent::DriveLost(drive_root.clone())]);
                    break;
                }

                waiting_for_io = true;
            }

            // Wait for I/O completion with timeout (100ms).
            let wait_result = WaitForSingleObject(h_event.as_raw(), 100);

            if wait_result.0 == 0 {
                // Event signaled - I/O completed.
                let result =
                    GetOverlappedResult(handle.as_raw(), &overlapped, &mut bytes_returned, false);

                if let Err(e) = &result {
                    // Handle became invalid - drive was likely unmounted.
                    log::error!(
                        "[DRIVE-WATCHER] GetOverlappedResult failed (drive likely unmounted): {}",
                        e
                    );
                    let _ = event_tx.send(vec![DriveWatcherEvent::DriveLost(drive_root.clone())]);
                    break;
                }

                if bytes_returned == 0 || bytes_returned as usize >= buffer.0.len() {
                    log::warn!(
                        "[DRIVE-WATCHER] Notification overflow on {:?}; invalidating {:?}",
                        drive_root,
                        current_prefix
                    );
                    coalesced.clear();
                    let _ = event_tx.send(vec![DriveWatcherEvent::PrefixInvalidated(
                        current_prefix.clone(),
                    )]);
                } else {
                    // Parse the notification buffer.
                    let events = parse_notify_buffer(&buffer.0[..bytes_returned as usize], &drive_root);

                    // Insert into coalescing set (deduplicates automatically)
                    // and filter by the currently watched prefix.
                    for event in events {
                        if event_matches_prefix(&event, &current_prefix) {
                            if coalesced.len() >= MAX_COALESCED_EVENTS {
                                let batch: Vec<DriveWatcherEvent> = coalesced.drain().collect();
                                let _ = event_tx.send(batch);
                                last_flush = std::time::Instant::now();
                            }
                            coalesced.insert(event);
                        }
                    }
                }

                // Reset event and mark I/O as complete.
                let _ = ResetEvent(h_event.as_raw());
                waiting_for_io = false;
            }

            // Flush coalesced events based on time or buffer pressure.
            let elapsed = last_flush.elapsed().as_millis() as u64;
            let should_flush = !coalesced.is_empty()
                && (elapsed >= COALESCE_INTERVAL_MS || coalesced.len() >= MAX_COALESCED_EVENTS);

            if should_flush {
                let batch: Vec<DriveWatcherEvent> = coalesced.drain().collect();
                let _ = event_tx.send(batch);
                last_flush = std::time::Instant::now();
            }
        }

        // Flush remaining events before shutdown.
        if !coalesced.is_empty() {
            let batch: Vec<DriveWatcherEvent> = coalesced.drain().collect();
            let _ = event_tx.send(batch);
        }

        // Cleanup: cancel pending I/O and wait for it to complete before
        // dropping the buffer.  Without this, CancelIoEx merely *schedules*
        // cancellation while Windows may still be writing into `buffer` /
        // `overlapped` — a use-after-free if they are dropped immediately.
        if waiting_for_io {
            let _ = CancelIoEx(handle.as_raw(), Some(&overlapped));
            let mut dummy = 0u32;
            let _ = GetOverlappedResult(handle.as_raw(), &overlapped, &mut dummy, true);
        } else {
            let _ = CancelIoEx(handle.as_raw(), None);
        }
        log::info!("[DRIVE-WATCHER] Thread shutdown complete");
    }
}
