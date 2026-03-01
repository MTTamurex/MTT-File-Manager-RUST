use std::fs::File;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FileIoPriorityHintInfo, SetFileInformationByHandle, FILE_FLAG_RANDOM_ACCESS,
    FILE_FLAG_SEQUENTIAL_SCAN, FILE_GENERIC_READ, FILE_IO_PRIORITY_HINT_INFO, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, PRIORITY_HINT,
};

pub fn open_sequential(path: &Path) -> std::io::Result<File> {
    let wide_path: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE:
    // Coexist with other processes that have the file open for writing
    // (e.g., browsers downloading, apps saving). Without SHARE_WRITE,
    // our read can interrupt/block an active download.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_SEQUENTIAL_SCAN,
            None,
        )
    };

    match handle {
        Ok(h) if h != INVALID_HANDLE_VALUE => Ok(unsafe { File::from_raw_handle(h.0) }),
        Ok(_) => Err(std::io::Error::last_os_error()),
        Err(e) => Err(std::io::Error::other(e.to_string())),
    }
}

pub fn open_random_access(path: &Path) -> std::io::Result<File> {
    let wide_path: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_RANDOM_ACCESS,
            None,
        )
    };

    match handle {
        Ok(h) if h != INVALID_HANDLE_VALUE => Ok(unsafe { File::from_raw_handle(h.0) }),
        Ok(_) => Err(std::io::Error::last_os_error()),
        Err(e) => Err(std::io::Error::other(e.to_string())),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileIoPriority {
    VeryLow = 0,
    Low = 1,
    Normal = 2,
}

pub fn set_file_io_priority<F: AsRawHandle>(
    file: &F,
    priority: FileIoPriority,
) -> std::io::Result<()> {
    let handle = HANDLE(file.as_raw_handle());
    let hint = FILE_IO_PRIORITY_HINT_INFO {
        PriorityHint: PRIORITY_HINT(priority as i32),
    };

    let result = unsafe {
        SetFileInformationByHandle(
            handle,
            FileIoPriorityHintInfo,
            &hint as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<FILE_IO_PRIORITY_HINT_INFO>() as u32,
        )
    };

    if result.is_ok() {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

pub fn open_sequential_low_priority(path: &Path) -> std::io::Result<File> {
    let file = open_sequential(path)?;
    let _ = set_file_io_priority(&file, FileIoPriority::Low);
    Ok(file)
}

pub fn open_sequential_background(path: &Path) -> std::io::Result<File> {
    let file = open_sequential(path)?;
    let _ = set_file_io_priority(&file, FileIoPriority::VeryLow);
    Ok(file)
}

/// Known extensions for incomplete download files.
///
/// Browsers and download managers write to these temp files while downloading.
/// Reading them for thumbnail extraction can corrupt the download or cause
/// sharing violations that interrupt the transfer.
const INCOMPLETE_DOWNLOAD_EXTENSIONS: &[&str] = &[
    "crdownload", // Chrome / Chromium / Edge
    "part",       // Firefox
    "download",   // Safari / older Firefox
    "partial",    // Internet Explorer / Wget
    "tmp",        // Generic temp (various apps)
    "opdownload", // Opera
    "crswap",     // Chrome swap file
];

/// Dynamic write-activity window for media files.
///
/// If a media file changed recently, we require a continuous stability window
/// (size + mtime unchanged across multiple extraction attempts) before allowing
/// COM-based thumbnail/metadata extraction.
const RECENT_WRITE_ACTIVITY_SECS: u64 = 300;

/// UI-thread fast guard window.
///
/// Keeps short-term protection right after MODIFY events, while avoiding long
/// post-download delays from fixed multi-minute cooldowns.
const FAST_RECENT_GUARD_SECS: u64 = 5;

/// Minimum continuous stability time required before treating a recently
/// changing media file as safe to read.
const MIN_STABLE_MEDIA_DURATION: Duration = Duration::from_secs(12);

const STABILITY_STATE_CAP: usize = 8192;
const STABILITY_STATE_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileReadSafety {
    Safe,
    IncompleteDownload,
    WriteLocked,
    RecentlyChanging,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStabilitySnapshot {
    len: u64,
    modified: SystemTime,
}

#[derive(Debug, Clone, Copy)]
struct FileStabilityState {
    last_len: u64,
    last_modified: SystemTime,
    stable_since: Instant,
    last_seen: Instant,
}

static FILE_STABILITY_CACHE: OnceLock<Mutex<std::collections::HashMap<PathBuf, FileStabilityState>>> =
    OnceLock::new();

fn get_file_stability_cache() -> &'static Mutex<std::collections::HashMap<PathBuf, FileStabilityState>> {
    FILE_STABILITY_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Returns `true` if the file extension indicates an incomplete download.
///
/// Covers Chrome (.crdownload), Firefox (.part), Safari (.download),
/// Opera (.opdownload), IE (.partial), and generic temp files (.tmp).
pub fn is_incomplete_download(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| {
            let lower = ext.to_ascii_lowercase();
            INCOMPLETE_DOWNLOAD_EXTENSIONS.contains(&lower.as_str())
        })
        .unwrap_or(false)
}

/// Returns `true` if another process currently holds the file open for WRITING.
///
/// Attempts `CreateFileW` with `GENERIC_READ` but WITHOUT `FILE_SHARE_WRITE`.
/// If this fails with `ERROR_SHARING_VIOLATION`, another process has the file
/// open for writing (e.g., an active download, encoding, save operation).
///
/// Cheap probe (~5µs on NVMe, single syscall, no I/O). The handle is closed
/// immediately if the open succeeds.
pub fn is_file_locked_for_write(path: &Path) -> bool {
    use windows::Win32::Foundation::CloseHandle;

    let wide_path: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // Try to open with READ access, sharing only READ (not WRITE).
    // If another process has the file open for WRITE, this will fail
    // with ERROR_SHARING_VIOLATION (0x20).
    let result = unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_DELETE, // deliberately NO FILE_SHARE_WRITE
            None,
            OPEN_EXISTING,
            windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    };

    match result {
        Ok(handle) => {
            // File is NOT locked for write — close the probe handle immediately
            let _ = unsafe { CloseHandle(handle) };
            false
        }
        Err(_) => {
            // ERROR_SHARING_VIOLATION or other error → treat as locked
            true
        }
    }
}

fn is_media_file(path: &Path) -> bool {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e,
        None => return false,
    };

    let lower = ext.to_ascii_lowercase();
    crate::infrastructure::windows::file_type::is_video_extension(&lower)
        || crate::infrastructure::windows::file_type::is_image_extension(&lower)
}

fn is_recently_modified_within(path: &Path, max_age_secs: u64) -> bool {
    if !is_media_file(path) {
        return false;
    }

    match std::fs::metadata(path).and_then(|m| m.modified()) {
        Ok(mtime) => mtime
            .elapsed()
            .map_or(false, |elapsed| elapsed.as_secs() < max_age_secs),
        Err(_) => false,
    }
}

fn read_stability_snapshot(path: &Path) -> Option<FileStabilitySnapshot> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    Some(FileStabilitySnapshot {
        len: metadata.len(),
        modified,
    })
}

fn is_stable_enough_across_attempts(path: &Path, snapshot: FileStabilitySnapshot) -> bool {
    let now = Instant::now();

    let Ok(mut cache) = get_file_stability_cache().lock() else {
        return false;
    };

    if cache.len() > STABILITY_STATE_CAP {
        cache.retain(|_, state| now.duration_since(state.last_seen) <= STABILITY_STATE_TTL);
    }

    let key = path.to_path_buf();
    let is_stable = match cache.get_mut(&key) {
        Some(state) => {
            if state.last_len != snapshot.len || state.last_modified != snapshot.modified {
                state.last_len = snapshot.len;
                state.last_modified = snapshot.modified;
                state.stable_since = now;
                state.last_seen = now;
                false
            } else {
                state.last_seen = now;
                now.duration_since(state.stable_since) >= MIN_STABLE_MEDIA_DURATION
            }
        }
        None => {
            cache.insert(
                key,
                FileStabilityState {
                    last_len: snapshot.len,
                    last_modified: snapshot.modified,
                    stable_since: now,
                    last_seen: now,
                },
            );
            false
        }
    };

    is_stable
}

/// Classify whether a file is currently safe to read for expensive media probes
/// (thumbnail extraction, metadata via COM/MF).
///
/// Worker-thread usage only: may perform lock probe and short sleep for
/// stability verification on recently modified media files.
pub fn classify_file_read_safety(path: &Path) -> FileReadSafety {
    if is_incomplete_download(path) {
        return FileReadSafety::IncompleteDownload;
    }

    if is_file_locked_for_write(path) {
        return FileReadSafety::WriteLocked;
    }

    if is_recently_modified_within(path, RECENT_WRITE_ACTIVITY_SECS) {
        let snapshot = match read_stability_snapshot(path) {
            Some(v) => v,
            None => return FileReadSafety::RecentlyChanging,
        };

        if !is_stable_enough_across_attempts(path, snapshot) {
            return FileReadSafety::RecentlyChanging;
        }
    }

    FileReadSafety::Safe
}

/// Combined check: returns `true` if the file should NOT be processed for
/// thumbnail extraction because it is an incomplete download or is actively
/// being written to by another process.
///
/// Three layers of defense:
/// 1. **Extension check**: `.crdownload`, `.part`, etc. (instant, zero cost)
/// 2. **Write-lock probe**: Detects another process holding WRITE access (5µs on NVMe,
///    but can take **hundreds of ms on virtual/network drives**). Only safe on worker threads.
/// 3. **Dynamic stability window**: For media files modified recently, we keep
///    per-path state (`len` + `mtime`) across attempts and only allow extraction
///    after a continuous stability period. This avoids race conditions where
///    piece-based writers briefly release file handles between writes.
///
/// **WARNING**: Contains `CreateFileW` probe (layer 2) that can block on
/// network/virtual drives. Use [`is_file_unsafe_to_read_fast`] on the UI thread.
pub fn is_file_unsafe_to_read(path: &Path) -> bool {
    match classify_file_read_safety(path) {
        FileReadSafety::Safe => false,
        FileReadSafety::IncompleteDownload => {
            log::debug!(
                "[FileFlags] Skipping incomplete download: {:?}",
                path.file_name()
            );
            true
        }
        FileReadSafety::WriteLocked => {
            log::debug!(
                "[FileFlags] Skipping file locked for write: {:?}",
                path.file_name()
            );
            true
        }
        FileReadSafety::RecentlyChanging => {
            log::debug!(
                "[FileFlags] Skipping unstable recently-modified media file: {:?}",
                path.file_name()
            );
            true
        }
    }
}

/// Lightweight version of [`is_file_unsafe_to_read`] safe for the UI thread.
///
/// Only performs cheap checks (extension + short recent-mtime window), skipping
/// the `CreateFileW` write-lock probe and stability delay which can block on
/// network/virtual drives.
/// This is sufficient for UI-thread decisions (watcher events, metadata requests)
/// because the full check will still be performed on the worker thread before any
/// file I/O occurs.
pub fn is_file_unsafe_to_read_fast(path: &Path) -> bool {
    if is_incomplete_download(path) {
        return true;
    }

    if is_recently_modified_within(path, FAST_RECENT_GUARD_SECS) {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn test_open_sequential() {
        let result = open_sequential(Path::new("C:\\Windows\\System32\\ntdll.dll"));
        assert!(result.is_ok());

        let mut file = result.unwrap();
        let mut buffer = [0u8; 2];
        assert!(file.read(&mut buffer).is_ok());
        assert_eq!(&buffer, b"MZ");
    }

    #[test]
    fn test_open_random_access() {
        let result = open_random_access(Path::new("C:\\Windows\\System32\\ntdll.dll"));
        assert!(result.is_ok());
    }
}
