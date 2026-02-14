//! I/O Priority management for optimized disk access.
//!
//! This module provides:
//! - SSD vs HDD detection
//! - Thread priority adjustment for background work
//! - Directory-grouped request scheduling to minimize seeks on HDDs

use std::path::Path;

mod detection;
mod grouped_queue;
mod threading;

pub use grouped_queue::DirectoryGroupedQueue;

/// Priority levels for I/O operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum IOPriority {
    /// Thumbnail visible on screen now; user is waiting.
    Interactive = 0,

    /// Thumbnail that will be visible soon (prefetch nearby items).
    #[default]
    Prefetch = 1,

    /// Background operations (folder covers, metadata discovery).
    Background = 2,
}

/// Checks whether a path belongs to a virtual drive (Cryptomator, Dokan, WinFSP, etc.).
pub fn is_virtual_drive_path(path: &Path) -> bool {
    detection::is_virtual_drive_path(path)
}

/// Detect if a path is on an SSD (no seek penalty) or HDD (has seek penalty).
pub fn is_ssd(path: &Path) -> bool {
    detection::is_ssd(path)
}

/// Invalidate cache for a specific drive (useful after configuration changes).
pub fn invalidate_drive_cache(drive_letter: char) {
    detection::invalidate_drive_cache(drive_letter)
}

/// Set the current thread's priority based on I/O priority level.
pub fn set_thread_priority(priority: IOPriority) {
    threading::set_thread_priority(priority)
}

/// Reset thread priority to normal (call after background work completes).
pub fn reset_thread_priority() {
    threading::reset_thread_priority()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_directory_grouped_queue_ssd() {
        let mut queue: DirectoryGroupedQueue<String> = DirectoryGroupedQueue::with_disk_type(true);

        queue.push(
            PathBuf::from("C:\\folder1\\file1.jpg"),
            IOPriority::Background,
            "file1".to_string(),
        );
        queue.push(
            PathBuf::from("C:\\folder2\\file2.jpg"),
            IOPriority::Interactive,
            "file2".to_string(),
        );
        queue.push(
            PathBuf::from("C:\\folder1\\file3.jpg"),
            IOPriority::Prefetch,
            "file3".to_string(),
        );

        // SSD mode: should return highest priority first regardless of directory
        assert_eq!(queue.pop(), Some("file2".to_string())); // Interactive
        assert_eq!(queue.pop(), Some("file3".to_string())); // Prefetch
        assert_eq!(queue.pop(), Some("file1".to_string())); // Background
        assert!(queue.is_empty());
    }

    #[test]
    fn test_directory_grouped_queue_hdd() {
        let mut queue: DirectoryGroupedQueue<String> = DirectoryGroupedQueue::with_disk_type(false);

        queue.push(
            PathBuf::from("C:\\folder1\\file1.jpg"),
            IOPriority::Prefetch,
            "file1".to_string(),
        );
        queue.push(
            PathBuf::from("C:\\folder2\\file2.jpg"),
            IOPriority::Interactive,
            "file2".to_string(),
        );
        queue.push(
            PathBuf::from("C:\\folder2\\file3.jpg"),
            IOPriority::Background,
            "file3".to_string(),
        );

        // HDD mode: should process folder2 items together after picking highest priority
        assert_eq!(queue.pop(), Some("file2".to_string())); // Interactive (folder2)
        assert_eq!(queue.pop(), Some("file3".to_string())); // Background (folder2 - same dir)
        assert_eq!(queue.pop(), Some("file1".to_string())); // Prefetch (folder1)
        assert!(queue.is_empty());
    }

    #[test]
    fn test_io_priority_ordering() {
        assert!(IOPriority::Interactive < IOPriority::Prefetch);
        assert!(IOPriority::Prefetch < IOPriority::Background);
    }
}

