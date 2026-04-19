//! Drive-wide file system watcher using ReadDirectoryChangesW
//!
//! This module provides optimized file system monitoring by watching the entire
//! drive root (e.g., C:\) instead of individual folders. This approach:
//! - Eliminates watcher setup/teardown overhead during navigation
//! - Prevents missed events during folder transitions
//! - Reduces handle usage (one handle per drive vs one per folder)
//! - Provides faster navigation with guaranteed change detection
//!
//! Uses ReadDirectoryChangesW on the entire drive instead of per folder.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

mod buffer_parser;
mod thread_loop;

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
    /// Watcher buffer overflowed and the prefix must be refreshed from disk
    PrefixInvalidated(PathBuf),
    /// Unknown/unsupported event
    Unknown(PathBuf),
    /// Drive became inaccessible (unmounted, disconnected)
    DriveLost(PathBuf),
}

/// Internal command for the watcher thread
#[derive(Debug, Clone)]
pub(super) enum WatcherCommand {
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
        // NOTE: Validation handle was removed to avoid blocking the UI thread.
        // CreateFileW on a sleeping HDD can stall for 500-2000ms.
        // The watcher thread handles open failures gracefully.

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

            thread_loop::watcher_thread_main(
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

    /// Poll events with per-frame limits and overflow draining.
    ///
    /// This method is intended for UI-thread consumers:
    /// - keeps at most `max_batches` channel batches
    /// - keeps at most `max_events` events
    /// - drains and drops any additional buffered events to prevent backlog bursts
    ///
    /// Returns `(events_kept, dropped_event_count)`.
    pub fn poll_events_limited(
        &self,
        max_batches: usize,
        max_events: usize,
    ) -> (Vec<DriveWatcherEvent>, usize) {
        let max_batches = max_batches.max(1);
        let max_events = max_events.max(1);

        let mut kept = Vec::with_capacity(max_events.min(1024));
        let mut dropped = 0usize;
        let mut batches_kept = 0usize;

        while let Ok(events) = self.event_receiver.try_recv() {
            let batch_len = events.len();

            if batches_kept >= max_batches || kept.len() >= max_events {
                dropped = dropped.saturating_add(batch_len);
                continue;
            }

            batches_kept = batches_kept.saturating_add(1);

            let remaining = max_events.saturating_sub(kept.len());
            if batch_len <= remaining {
                kept.extend(events);
            } else {
                kept.extend(events.into_iter().take(remaining));
                dropped = dropped.saturating_add(batch_len.saturating_sub(remaining));
            }
        }

        (kept, dropped)
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
            .as_os_str()
            .encode_wide()
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
    /// NOTE: On Windows, `Path::components().next()` returns "C:" (without backslash),
    /// which means "current directory on drive C" - NOT the root. We must append `\`
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

#[cfg(test)]
mod tests {
    use super::buffer_parser::event_matches_prefix;
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

        let invalidated = DriveWatcherEvent::PrefixInvalidated(PathBuf::from("C:\\Users\\Test"));
        assert!(event_matches_prefix(
            &invalidated,
            Path::new("C:\\Users\\Test\\Nested")
        ));
    }
}
