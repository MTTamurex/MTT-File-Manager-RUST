//! I/O Priority management for optimized disk access
//!
//! This module provides:
//! - SSD vs HDD detection
//! - Thread priority adjustment for background work
//! - Directory-grouped request scheduling to minimize seeks on HDDs

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rustc_hash::FxHashMap;

/// Cache of disk type detection results (drive letter -> is_ssd)
static DISK_TYPE_CACHE: OnceLock<std::sync::Mutex<FxHashMap<char, bool>>> = OnceLock::new();

fn get_disk_cache() -> &'static std::sync::Mutex<FxHashMap<char, bool>> {
    DISK_TYPE_CACHE.get_or_init(|| std::sync::Mutex::new(FxHashMap::default()))
}

/// Detects if a drive is a virtual Cryptomator drive
///
/// Cryptomator mounts encrypted vaults as virtual drives using CryptoFS.
/// These should be treated as HDDs if the underlying storage is an HDD.
fn is_virtual_drive(drive_letter: char) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetVolumeInformationW;

    let root_path = format!("{}:\\", drive_letter);
    let wide_path: Vec<u16> = root_path.encode_utf16().chain(std::iter::once(0)).collect();

    let mut volume_name = [0u16; 261];
    let mut file_system_name = [0u16; 261];
    let mut serial_number: u32 = 0;
    let mut max_component_len: u32 = 0;
    let mut fs_flags: u32 = 0;

    let ok = unsafe {
        GetVolumeInformationW(
            PCWSTR(wide_path.as_ptr()),
            Some(&mut volume_name),
            Some(&mut serial_number),
            Some(&mut max_component_len),
            Some(&mut fs_flags),
            Some(&mut file_system_name),
        )
    };

    if !ok.is_ok() {
        return false;
    }

    let volume_len = volume_name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(volume_name.len());
    let fs_len = file_system_name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(file_system_name.len());

    let volume = String::from_utf16_lossy(&volume_name[..volume_len]).to_lowercase();
    let file_system = String::from_utf16_lossy(&file_system_name[..fs_len]).to_lowercase();

    // Detect virtual drive indicators (CryptoFS is the file system name used by Cryptomator on Windows)
    let is_virtual = volume.contains("cryptomator")
        || file_system.contains("cryptofs")
        || file_system.contains("dokan")
        || file_system.contains("winfsp")
        || file_system == "fuse";

    if is_virtual {
        eprintln!(
            "[IO] Virtual drive detected: {}:\\ (Volume: '{}', FS: '{}') - treating as HDD",
            drive_letter, volume, file_system
        );
    }

    is_virtual
}

/// Priority levels for I/O operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IOPriority {
    /// Thumbnail visible on screen NOW - user is waiting
    /// Processed with normal thread priority
    Interactive = 0,

    /// Thumbnail that will be visible soon (prefetch nearby items)
    /// Processed with slightly lower priority
    Prefetch = 1,

    /// Background operations (folder covers, metadata discovery)
    /// Processed with lowest priority, yields to other I/O
    Background = 2,
}

impl Default for IOPriority {
    fn default() -> Self {
        IOPriority::Prefetch
    }
}

/// Detect if a path is on an SSD (no seek penalty) or HDD (has seek penalty)
///
/// Uses Windows DeviceIoControl with IOCTL_STORAGE_QUERY_PROPERTY to check
/// the StorageDeviceSeekPenaltyProperty. SSDs return IncursSeekPenalty = false.
///
/// Special handling for virtual drives (like Cryptomator):
/// - Virtual drives are treated as HDDs since the underlying encrypted storage
///   is typically on HDDs and benefits from seek-minimizing strategies
///
/// Results are cached per drive letter for performance.
pub fn is_ssd(path: &Path) -> bool {
    // Extract drive letter (e.g., "C:" from "C:\Users\...")
    let drive_letter = match path.to_str() {
        Some(s) if s.len() >= 2 && s.chars().nth(1) == Some(':') => {
            s.chars().next().unwrap().to_ascii_uppercase()
        }
        _ => return true, // Assume SSD for network paths, etc.
    };

    // Check cache first
    if let Ok(cache) = get_disk_cache().lock() {
        if let Some(&is_ssd) = cache.get(&drive_letter) {
            return is_ssd;
        }
    }

    // Check if it's a virtual drive (Cryptomator, Dokan, WinFsp)
    // Virtual drives should be treated as HDDs for optimization purposes
    if is_virtual_drive(drive_letter) {
        if let Ok(mut cache) = get_disk_cache().lock() {
            cache.insert(drive_letter, false);
        }
        return false;
    }

    // Query Windows for disk type
    let result = query_disk_seek_penalty(drive_letter);

    // Cache the result
    if let Ok(mut cache) = get_disk_cache().lock() {
        cache.insert(drive_letter, result);
    }

    result
}

/// Query Windows for whether a disk has seek penalty (HDD) or not (SSD)
fn query_disk_seek_penalty(drive_letter: char) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Ioctl::IOCTL_STORAGE_QUERY_PROPERTY;
    use windows::Win32::System::IO::DeviceIoControl;

    // Construct path like "\\.\C:"
    let device_path = format!("\\\\.\\{}:", drive_letter);
    let wide_path: Vec<u16> = device_path.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        // Open handle to the physical drive
        let handle = CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            0, // No access needed, just query
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        );

        let handle = match handle {
            Ok(h) if h != INVALID_HANDLE_VALUE => h,
            _ => return true, // Assume SSD on error (safer for performance)
        };

        // StorageDeviceSeekPenaltyProperty = 7
        const STORAGE_DEVICE_SEEK_PENALTY_PROPERTY: u32 = 7;
        // PropertyStandardQuery = 0
        const PROPERTY_STANDARD_QUERY: u32 = 0;

        #[repr(C)]
        struct StoragePropertyQuery {
            property_id: u32,
            query_type: u32,
            additional_parameters: [u8; 1],
        }

        #[repr(C)]
        struct DeviceSeekPenaltyDescriptor {
            version: u32,
            size: u32,
            incurs_seek_penalty: u8, // BOOLEAN
        }

        let query = StoragePropertyQuery {
            property_id: STORAGE_DEVICE_SEEK_PENALTY_PROPERTY,
            query_type: PROPERTY_STANDARD_QUERY,
            additional_parameters: [0],
        };

        let mut result = DeviceSeekPenaltyDescriptor {
            version: 0,
            size: 0,
            incurs_seek_penalty: 1, // Default to HDD (seek penalty)
        };

        let mut bytes_returned: u32 = 0;

        let success = DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&query as *const _ as *const std::ffi::c_void),
            std::mem::size_of::<StoragePropertyQuery>() as u32,
            Some(&mut result as *mut _ as *mut std::ffi::c_void),
            std::mem::size_of::<DeviceSeekPenaltyDescriptor>() as u32,
            Some(&mut bytes_returned),
            None,
        );

        let _ = CloseHandle(handle);

        if success.is_ok() && bytes_returned > 0 {
            // incurs_seek_penalty == 0 means SSD (no seek penalty)
            // incurs_seek_penalty == 1 means HDD (has seek penalty)
            result.incurs_seek_penalty == 0
        } else {
            // Assume SSD on query failure (safer default)
            true
        }
    }
}

/// Set the current thread's priority based on I/O priority level
///
/// - Interactive: Above normal (faster response)
/// - Prefetch: Normal
/// - Background: Lowest + Background mode (minimal I/O impact)
pub fn set_thread_priority(priority: IOPriority) {
    use windows::Win32::System::Threading::*;

    unsafe {
        let thread = GetCurrentThread();

        match priority {
            IOPriority::Interactive => {
                // Slightly elevated for responsive thumbnails
                let _ = SetThreadPriority(thread, THREAD_PRIORITY_ABOVE_NORMAL);
            }
            IOPriority::Prefetch => {
                // Normal priority
                let _ = SetThreadPriority(thread, THREAD_PRIORITY_NORMAL);
            }
            IOPriority::Background => {
                // Lowest priority - yields to other work
                let _ = SetThreadPriority(thread, THREAD_PRIORITY_LOWEST);

                // Enable background processing mode (Windows Vista+)
                // This tells the OS to give this thread minimal I/O priority
                let _ = SetThreadPriority(thread, THREAD_MODE_BACKGROUND_BEGIN);
            }
        }
    }
}

/// Reset thread priority to normal (call after background work completes)
pub fn reset_thread_priority() {
    use windows::Win32::System::Threading::*;

    unsafe {
        let thread = GetCurrentThread();

        // Exit background mode if active
        let _ = SetThreadPriority(thread, THREAD_MODE_BACKGROUND_END);

        // Reset to normal
        let _ = SetThreadPriority(thread, THREAD_PRIORITY_NORMAL);
    }
}

/// Groups requests by directory to minimize disk seeks on HDDs
///
/// For SSDs, this just returns items sorted by priority.
/// For HDDs, items are grouped by parent directory so that sequential reads
/// from the same folder happen together, minimizing expensive seek operations.
pub struct DirectoryGroupedQueue<T> {
    /// Items grouped by parent directory
    by_directory: FxHashMap<PathBuf, Vec<(IOPriority, T)>>,

    /// Whether we're on an SSD (skip grouping optimization)
    is_ssd: bool,

    /// Current directory being processed (for HDD locality optimization)
    current_directory: Option<PathBuf>,
}

impl<T> DirectoryGroupedQueue<T> {
    /// Create a new queue, detecting disk type from the given path
    pub fn new(sample_path: &Path) -> Self {
        Self {
            by_directory: FxHashMap::default(),
            is_ssd: is_ssd(sample_path),
            current_directory: None,
        }
    }

    /// Create a queue with explicit SSD/HDD mode
    pub fn with_disk_type(is_ssd: bool) -> Self {
        Self {
            by_directory: FxHashMap::default(),
            is_ssd,
            current_directory: None,
        }
    }

    /// Add an item to the queue
    pub fn push(&mut self, path: PathBuf, priority: IOPriority, item: T) {
        let parent = path.parent().unwrap_or(&path).to_path_buf();
        self.by_directory
            .entry(parent)
            .or_insert_with(Vec::new)
            .push((priority, item));
    }

    /// Get the next item, optimizing for disk locality on HDDs
    pub fn pop(&mut self) -> Option<T> {
        if self.by_directory.is_empty() {
            return None;
        }

        if self.is_ssd {
            // SSD: Just get highest priority item from any directory
            self.pop_highest_priority()
        } else {
            // HDD: Prefer items from current directory to minimize seeks
            self.pop_with_locality()
        }
    }

    /// Pop highest priority item regardless of directory (SSD mode)
    fn pop_highest_priority(&mut self) -> Option<T> {
        // Find directory with highest priority item
        let best_dir = self
            .by_directory
            .iter()
            .filter(|(_, items)| !items.is_empty())
            .min_by_key(|(_, items)| items.iter().map(|(p, _)| *p).min().unwrap_or(IOPriority::Background))
            .map(|(dir, _)| dir.clone())?;

        self.pop_from_directory(&best_dir)
    }

    /// Pop item with locality preference (HDD mode)
    fn pop_with_locality(&mut self) -> Option<T> {
        // If we have a current directory with items, continue there
        if let Some(ref dir) = self.current_directory.clone() {
            if let Some(items) = self.by_directory.get(dir) {
                if !items.is_empty() {
                    return self.pop_from_directory(dir);
                }
            }
        }

        // Find directory with highest priority item
        let best_dir = self
            .by_directory
            .iter()
            .filter(|(_, items)| !items.is_empty())
            .min_by_key(|(_, items)| items.iter().map(|(p, _)| *p).min().unwrap_or(IOPriority::Background))
            .map(|(dir, _)| dir.clone())?;

        self.current_directory = Some(best_dir.clone());
        self.pop_from_directory(&best_dir)
    }

    /// Pop highest priority item from a specific directory
    fn pop_from_directory(&mut self, dir: &PathBuf) -> Option<T> {
        let items = self.by_directory.get_mut(dir)?;

        if items.is_empty() {
            self.by_directory.remove(dir);
            return None;
        }

        // Find index of highest priority item
        let best_idx = items
            .iter()
            .enumerate()
            .min_by_key(|(_, (p, _))| *p)
            .map(|(idx, _)| idx)?;

        let (_, item) = items.swap_remove(best_idx);

        // Clean up empty directories
        if items.is_empty() {
            self.by_directory.remove(dir);
            if self.current_directory.as_ref() == Some(dir) {
                self.current_directory = None;
            }
        }

        Some(item)
    }

    /// Check if queue is empty
    pub fn is_empty(&self) -> bool {
        self.by_directory.values().all(|v| v.is_empty())
    }

    /// Get total item count
    pub fn len(&self) -> usize {
        self.by_directory.values().map(|v| v.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
