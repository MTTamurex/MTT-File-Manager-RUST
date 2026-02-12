//! Drive-wide file system watcher using ReadDirectoryChangesW
//!
//! This module provides optimized file system monitoring by watching the entire
//! drive root (e.g., C:\) instead of individual folders. This approach:
//! - Eliminates watcher setup/teardown overhead during navigation
//! - Prevents missed events during folder transitions
//! - Reduces handle usage (one handle per drive vs one per folder)
//! - Provides faster navigation with guaranteed change detection
//!
//! Based on File Pilot's approach: "Stabilized directory change tracking by using
//! ReadDirectoryChanges on the entire drive instead of per folder."

use std::collections::HashSet;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadDirectoryChangesW, FILE_ACTION_ADDED, FILE_ACTION_MODIFIED,
    FILE_ACTION_REMOVED, FILE_ACTION_RENAMED_NEW_NAME, FILE_ACTION_RENAMED_OLD_NAME,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY,
    FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_CREATION, FILE_NOTIFY_CHANGE_DIR_NAME,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
    FILE_NOTIFY_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult};

/// Buffer size for directory change notifications (64KB is the typical max)
const BUFFER_SIZE: usize = 65536;

/// Maximum events to keep in the dedup buffer before flushing.
/// When exceeded, events are coalesced into a bulk invalidation.
const MAX_COALESCED_EVENTS: usize = 500;

/// Minimum interval (ms) between sending event batches to the UI thread.
/// This prevents flooding the channel during OneDrive dehydration storms.
const COALESCE_INTERVAL_MS: u64 = 200;

/// Events that can be reported by the drive watcher
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DriveWatcherEvent {
    /// File or folder was created
    Created(PathBuf),
    /// File or folder was deleted
    Deleted(PathBuf),
    /// File or folder was modified (content or attributes)
    Modified(PathBuf),
    /// File or folder was renamed (old path, new path)
    Renamed(PathBuf, PathBuf),
    /// Unknown/unsupported event
    Unknown(PathBuf),
    /// Drive became inaccessible (unmounted, disconnected)
    DriveLost(PathBuf),
}

/// Internal command for the watcher thread
#[derive(Debug, Clone)]
enum WatcherCommand {
    /// Update the filter prefix (when user navigates to a different folder)
    UpdatePrefix(PathBuf),
    /// Shutdown the watcher
    Shutdown,
}

/// Drive-wide file system watcher
///
/// Watches an entire drive (e.g., C:\) and filters events to report
/// only those affecting the currently monitored prefix path.
pub struct DriveWatcher {
    /// Handle to the background thread
    _thread: Option<JoinHandle<()>>,
    /// Channel to send commands to the watcher thread
    command_sender: std::sync::mpsc::Sender<WatcherCommand>,
    /// Channel to receive events from the watcher thread
    event_receiver: std::sync::mpsc::Receiver<Vec<DriveWatcherEvent>>,
    /// Current watched path prefix (for filtering)
    current_prefix: Arc<Mutex<PathBuf>>,
    /// Shutdown flag
    shutdown: Arc<AtomicBool>,
}

impl DriveWatcher {
    /// Create a new drive watcher for the specified drive root
    ///
    /// # Arguments
    /// * `drive_root` - The drive to watch (e.g., "C:\")
    /// * `initial_prefix` - Initial path prefix to filter events (e.g., "C:\Users\Name\Documents")
    ///
    /// # Returns
    /// * `Some(DriveWatcher)` if the watcher was successfully created
    /// * `None` if the drive couldn't be opened or is not accessible
    pub fn new(drive_root: PathBuf, initial_prefix: PathBuf) -> Option<Self> {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let prefix = Arc::new(Mutex::new(initial_prefix.clone()));
        let shutdown = Arc::new(AtomicBool::new(false));

        let shutdown_clone = Arc::clone(&shutdown);
        let initial_prefix_for_thread = initial_prefix.clone();

        // Open the drive handle in the main thread to validate early
        // We pass the path to the thread and open it there to avoid Send issues with HANDLE
        let drive_root_clone = drive_root.clone();

        let thread = thread::spawn(move || {
            // Open handle inside the thread to avoid Send issues
            let Some(handle) = Self::open_drive_handle(&drive_root_clone) else {
                log::error!(
                    "[DRIVE-WATCHER] Failed to open drive: {:?}",
                    drive_root_clone
                );
                return;
            };

            watcher_thread_main(
                handle,
                drive_root_clone,
                cmd_rx,
                event_tx,
                shutdown_clone,
                initial_prefix_for_thread,
            );
        });

        Some(Self {
            _thread: Some(thread),
            command_sender: cmd_tx,
            event_receiver: event_rx,
            current_prefix: prefix,
            shutdown,
        })
    }

    /// Update the path prefix to filter events
    ///
    /// Call this when the user navigates to a different folder.
    /// The watcher continues monitoring the drive but only reports
    /// events within the new prefix path.
    pub fn update_prefix(&self, new_prefix: PathBuf) {
        // Update the shared prefix first
        if let Ok(mut prefix) = self.current_prefix.lock() {
            *prefix = new_prefix.clone();
        }
        // Notify the watcher thread
        let _ = self
            .command_sender
            .send(WatcherCommand::UpdatePrefix(new_prefix));
    }

    /// Poll for new events
    ///
    /// Returns a vector of events that occurred since the last poll.
    /// Events are pre-deduplicated and coalesced by the watcher thread,
    /// so this method is lightweight and safe to call on the UI thread.
    pub fn poll_events(&self) -> Vec<DriveWatcherEvent> {
        // Collect all available pre-coalesced batches (non-blocking)
        let mut all_events = Vec::new();
        while let Ok(events) = self.event_receiver.try_recv() {
            all_events.extend(events);
        }
        all_events
    }

    /// Check if the watcher is still running
    pub fn is_running(&self) -> bool {
        !self.shutdown.load(Ordering::Relaxed)
    }

    /// Get the current prefix being watched
    pub fn current_prefix(&self) -> PathBuf {
        self.current_prefix
            .lock()
            .map(|p| p.clone())
            .unwrap_or_default()
    }

    /// Open a handle to the drive for directory change monitoring
    fn open_drive_handle(drive_root: &Path) -> Option<HANDLE> {
        let wide_path: Vec<u16> = drive_root
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            // NOTE: FILE_FLAG_BACKUP_SEMANTICS is required for directory handles
            // Removing it breaks ReadDirectoryChangesW functionality
            let handle = CreateFileW(
                PCWSTR(wide_path.as_ptr()),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                None,
            );

            match handle {
                Ok(h) if h != INVALID_HANDLE_VALUE => Some(h),
                _ => {
                    log::error!("[DRIVE-WATCHER] Failed to open drive: {:?}", drive_root);
                    None
                }
            }
        }
    }

    /// Extract the drive root from a full path
    ///
    /// Example: "C:\Users\Name" -> "C:\"
    ///
    /// NOTE: On Windows, `Path::components().next()` returns `"C:"` (without backslash),
    /// which means "current directory on drive C" — NOT the root. We must append `\`
    /// so that `CreateFileW` opens the actual drive root for `ReadDirectoryChangesW`.
    pub fn extract_drive_root(path: &Path) -> Option<PathBuf> {
        let s = path.to_string_lossy();
        if s.len() >= 2 && s.as_bytes()[1] == b':' {
            Some(PathBuf::from(format!("{}\\", &s[..2])))
        } else {
            None
        }
    }
}

impl Drop for DriveWatcher {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = self.command_sender.send(WatcherCommand::Shutdown);

        if let Some(thread) = self._thread.take() {
            let _ = thread.join();
        }
    }
}

/// Main watcher thread function
///
/// ARCHITECTURE (inspired by Files app):
/// - Events are coalesced in a HashSet to deduplicate (same path → one event)
/// - Batches are flushed at most every COALESCE_INTERVAL_MS (200ms)
/// - When the buffer exceeds MAX_COALESCED_EVENTS, it's flushed immediately
///   to prevent unbounded memory growth during OneDrive dehydration storms
/// - This ensures the UI thread NEVER receives unbounded event lists
fn watcher_thread_main(
    handle: HANDLE,
    drive_root: PathBuf,
    command_rx: std::sync::mpsc::Receiver<WatcherCommand>,
    event_tx: std::sync::mpsc::Sender<Vec<DriveWatcherEvent>>,
    shutdown: Arc<AtomicBool>,
    mut current_prefix: PathBuf,
) {
    log::info!("[DRIVE-WATCHER] Thread started for drive: {:?}", drive_root);

    unsafe {
        // Create events for overlapped I/O
        let h_event = match CreateEventW(None, true, false, None) {
            Ok(event) => event,
            Err(e) => {
                log::error!("[DRIVE-WATCHER] Failed to create event: {}", e);
                let _ = CloseHandle(handle);
                return;
            }
        };

        // Buffer for directory change notifications
        let mut buffer: Vec<u8> = vec![0; BUFFER_SIZE];
        let mut overlapped = std::mem::zeroed::<windows::Win32::System::IO::OVERLAPPED>();
        overlapped.hEvent = h_event;

        // Coalescing state: events are deduplicated here before sending
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

            // Start async read if not already pending
            if !waiting_for_io {
                buffer.fill(0);
                let result = ReadDirectoryChangesW(
                    handle,
                    buffer.as_mut_ptr() as *mut _,
                    buffer.len() as u32,
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

            // Wait for I/O completion with timeout (100ms)
            let wait_result = WaitForSingleObject(h_event, 100);

            if wait_result.0 == 0 {
                // Event signaled - I/O completed
                let result = GetOverlappedResult(handle, &overlapped, &mut bytes_returned, false);

                if let Err(e) = &result {
                    // Handle became invalid — drive was likely unmounted
                    log::error!(
                        "[DRIVE-WATCHER] GetOverlappedResult failed (drive likely unmounted): {}",
                        e
                    );
                    let _ = event_tx.send(vec![DriveWatcherEvent::DriveLost(drive_root.clone())]);
                    break;
                }

                if bytes_returned > 0 {
                    // Parse the notification buffer
                    let events =
                        parse_notify_buffer(&buffer[..bytes_returned as usize], &drive_root);

                    // Insert into coalescing set (deduplicates automatically)
                    // and filter by the currently watched prefix.
                    for event in events {
                        if event_matches_prefix(&event, &current_prefix) {
                            coalesced.insert(event);
                        }
                    }
                }

                // Reset event and mark I/O as complete
                let _ = ResetEvent(h_event);
                waiting_for_io = false;
            }

            // Flush coalesced events based on time or buffer pressure
            let elapsed = last_flush.elapsed().as_millis() as u64;
            let should_flush = !coalesced.is_empty()
                && (elapsed >= COALESCE_INTERVAL_MS || coalesced.len() >= MAX_COALESCED_EVENTS);

            if should_flush {
                let batch: Vec<DriveWatcherEvent> = coalesced.drain().collect();
                let _ = event_tx.send(batch);
                last_flush = std::time::Instant::now();
            }
        }

        // Flush remaining events before shutdown
        if !coalesced.is_empty() {
            let batch: Vec<DriveWatcherEvent> = coalesced.drain().collect();
            let _ = event_tx.send(batch);
        }

        // Cleanup
        let _ = CancelIoEx(handle, None);
        let _ = CloseHandle(handle);
        let _ = CloseHandle(h_event);
        log::info!("[DRIVE-WATCHER] Thread shutdown complete");
    }
}

/// Parse FILE_NOTIFY_INFORMATION buffer into events
fn parse_notify_buffer(buffer: &[u8], drive_root: &Path) -> Vec<DriveWatcherEvent> {
    let mut events = Vec::new();
    let mut offset = 0usize;
    let mut pending_rename_old: Option<PathBuf> = None;

    // Ensure drive_root ends with backslash for proper path construction
    let drive_root_str = drive_root.to_string_lossy();
    let drive_root_normalized = if drive_root_str.ends_with('\\') {
        drive_root_str.to_string()
    } else {
        format!("{}\\", drive_root_str)
    };

    unsafe {
        loop {
            if offset + std::mem::size_of::<FILE_NOTIFY_INFORMATION>() > buffer.len() {
                break;
            }

            let info = &*(buffer.as_ptr().add(offset) as *const FILE_NOTIFY_INFORMATION);

            // Extract filename (comes as relative path from watched directory)
            let name_len = info.FileNameLength as usize / 2;
            let name_ptr = info.FileName.as_ptr();
            let name_slice = std::slice::from_raw_parts(name_ptr, name_len);
            let filename = OsString::from_wide(name_slice);
            let filename_str = filename.to_string_lossy();

            // Build full path - manually concatenate to avoid Path::join issues
            // FILE_NOTIFY_INFORMATION returns paths like "file.txt" or "folder\file.txt"
            // We need to prepend the drive root
            let full_path_str = format!("{}{}", drive_root_normalized, filename_str);
            let full_path = PathBuf::from(full_path_str);

            // Determine event type using FILE_ACTION constants
            match info.Action {
                FILE_ACTION_ADDED => events.push(DriveWatcherEvent::Created(full_path)),
                FILE_ACTION_REMOVED => events.push(DriveWatcherEvent::Deleted(full_path)),
                FILE_ACTION_MODIFIED => events.push(DriveWatcherEvent::Modified(full_path)),
                FILE_ACTION_RENAMED_OLD_NAME => {
                    // Rename notifications arrive as old-name + new-name pairs.
                    // Keep old-name pending and emit only when new-name arrives.
                    if let Some(unpaired_old) = pending_rename_old.replace(full_path) {
                        // Two OLD events in a row (rare): flush previous one conservatively.
                        events.push(DriveWatcherEvent::Renamed(
                            unpaired_old.clone(),
                            unpaired_old,
                        ));
                    }
                }
                FILE_ACTION_RENAMED_NEW_NAME => {
                    if let Some(old_path) = pending_rename_old.take() {
                        events.push(DriveWatcherEvent::Renamed(old_path, full_path));
                    } else {
                        // Defensive fallback for unmatched NEW event.
                        events.push(DriveWatcherEvent::Renamed(full_path.clone(), full_path));
                    }
                }
                _ => events.push(DriveWatcherEvent::Unknown(full_path)),
            }

            // Move to next entry
            if info.NextEntryOffset == 0 {
                break;
            }
            offset += info.NextEntryOffset as usize;
        }
    }

    // Defensive fallback: unmatched OLD rename at buffer end.
    if let Some(old_path) = pending_rename_old.take() {
        events.push(DriveWatcherEvent::Renamed(old_path.clone(), old_path));
    }

    events
}

fn path_matches_prefix(path: &Path, prefix: &Path) -> bool {
    // Normalize both paths for comparison
    let path_str_raw = path.to_string_lossy().to_lowercase();
    let prefix_str_raw = prefix.to_string_lossy().to_lowercase();

    if prefix_str_raw.is_empty() {
        return true;
    }

    let path_str = path_str_raw.strip_prefix(r"\\?\").unwrap_or(&path_str_raw);
    let prefix_str = prefix_str_raw
        .strip_prefix(r"\\?\")
        .unwrap_or(&prefix_str_raw);

    // Ensure both end with backslash for proper prefix matching
    let prefix_normalized = if prefix_str.ends_with('\\') {
        prefix_str.to_string()
    } else {
        format!("{}\\", prefix_str)
    };

    path_str.starts_with(&prefix_normalized)
        // Special case: if prefix is drive root (e.g., "d:\\"),
        // any path on that drive matches.
        || (prefix_normalized.len() == 3 && path_str.starts_with(&prefix_normalized[..2]))
}

/// Check if an event matches the current prefix.
fn event_matches_prefix(event: &DriveWatcherEvent, prefix: &Path) -> bool {
    match event {
        DriveWatcherEvent::DriveLost(_) => true, // Always propagate
        DriveWatcherEvent::Created(p)
        | DriveWatcherEvent::Deleted(p)
        | DriveWatcherEvent::Modified(p)
        | DriveWatcherEvent::Unknown(p) => path_matches_prefix(p, prefix),
        DriveWatcherEvent::Renamed(old, new) => {
            path_matches_prefix(old, prefix) || path_matches_prefix(new, prefix)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_drive_root() {
        assert_eq!(
            DriveWatcher::extract_drive_root(Path::new("C:\\Users\\Test")),
            Some(PathBuf::from("C:\\"))
        );
        assert_eq!(
            DriveWatcher::extract_drive_root(Path::new("D:\\")),
            Some(PathBuf::from("D:\\"))
        );
    }

    #[test]
    fn test_event_matches_prefix() {
        let event = DriveWatcherEvent::Created(PathBuf::from("C:\\Users\\Test\\file.txt"));
        assert!(event_matches_prefix(&event, Path::new("C:\\Users\\Test")));
        assert!(!event_matches_prefix(&event, Path::new("C:\\Users\\Other")));
        assert!(!event_matches_prefix(&event, Path::new("D:\\Users\\Test")));
    }
}
