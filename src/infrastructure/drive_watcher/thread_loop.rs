use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    ReadDirectoryChangesW, FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_CREATION,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
    FILE_NOTIFY_CHANGE_SIZE,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};

use super::buffer_parser::{event_matches_prefix, parse_notify_buffer};
use super::{DriveWatcherEvent, WatcherCommand};

/// Buffer size for directory change notifications (64KB is the typical max).
const BUFFER_SIZE: usize = 65536;

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
        // Create events for overlapped I/O.
        let h_event = match CreateEventW(None, true, false, None) {
            Ok(event) => event,
            Err(e) => {
                log::error!("[DRIVE-WATCHER] Failed to create event: {}", e);
                let _ = CloseHandle(handle);
                return;
            }
        };

        // Buffer for directory change notifications.
        let mut buffer = AlignedBuffer([0u8; BUFFER_SIZE]);
        let mut overlapped = std::mem::zeroed::<OVERLAPPED>();
        overlapped.hEvent = h_event;

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
                let result = ReadDirectoryChangesW(
                    handle,
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
            let wait_result = WaitForSingleObject(h_event, 100);

            if wait_result.0 == 0 {
                // Event signaled - I/O completed.
                let result = GetOverlappedResult(handle, &overlapped, &mut bytes_returned, false);

                if let Err(e) = &result {
                    // Handle became invalid - drive was likely unmounted.
                    log::error!(
                        "[DRIVE-WATCHER] GetOverlappedResult failed (drive likely unmounted): {}",
                        e
                    );
                    let _ = event_tx.send(vec![DriveWatcherEvent::DriveLost(drive_root.clone())]);
                    break;
                }

                if bytes_returned > 0 {
                    // Parse the notification buffer.
                    let events = parse_notify_buffer(&buffer.0[..bytes_returned as usize], &drive_root);

                    // Insert into coalescing set (deduplicates automatically)
                    // and filter by the currently watched prefix.
                    for event in events {
                        if event_matches_prefix(&event, &current_prefix) {
                            coalesced.insert(event);
                        }
                    }
                }

                // Reset event and mark I/O as complete.
                let _ = ResetEvent(h_event);
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

        // Cleanup.
        let _ = CancelIoEx(handle, None);
        let _ = CloseHandle(handle);
        let _ = CloseHandle(h_event);
        log::info!("[DRIVE-WATCHER] Thread shutdown complete");
    }
}
