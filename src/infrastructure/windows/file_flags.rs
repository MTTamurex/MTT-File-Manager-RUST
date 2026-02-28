use std::fs::File;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;
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

/// Combined check: returns `true` if the file should NOT be processed for
/// thumbnail extraction because it is an incomplete download or is actively
/// being written to by another process.
///
/// Three layers of defense:
/// 1. **Extension check**: `.crdownload`, `.part`, etc. (instant, zero cost)
/// 2. **Write-lock probe**: Detects another process holding WRITE access (5µs on NVMe,
///    but can take **hundreds of ms on virtual/network drives**). Only safe on worker threads.
/// 3. **Recently-modified guard**: If the file's mtime is within the last
///    2 minutes, assume it's still being written to. This catches race
///    conditions where torrent/encoding clients briefly release the file handle
///    between piece writes — our probe succeeds momentarily, but then COM APIs
///    (IShellItemImageFactory, MFCreateSourceReaderFromURL) open the file
///    internally WITHOUT `FILE_SHARE_WRITE`, blocking the writer.
///
///    The 2-minute window is intentionally generous: torrent clients can pause
///    between pieces for 30-90 seconds on slow connections, and the cost of
///    waiting an extra minute for a thumbnail is negligible compared to killing
///    a multi-GB download.
///
/// **WARNING**: Contains `CreateFileW` probe (layer 2) that can block on
/// network/virtual drives. Use [`is_file_unsafe_to_read_fast`] on the UI thread.
pub fn is_file_unsafe_to_read(path: &Path) -> bool {
    if is_incomplete_download(path) {
        log::debug!(
            "[FileFlags] Skipping incomplete download: {:?}",
            path.file_name()
        );
        return true;
    }

    if is_file_locked_for_write(path) {
        log::debug!(
            "[FileFlags] Skipping file locked for write: {:?}",
            path.file_name()
        );
        return true;
    }

    if is_recently_modified(path) {
        log::debug!(
            "[FileFlags] Skipping recently-modified file (likely active download/encode): {:?}",
            path.file_name()
        );
        return true;
    }

    false
}

/// Lightweight version of [`is_file_unsafe_to_read`] safe for the UI thread.
///
/// Only performs cheap checks (extension + mtime), skipping the `CreateFileW`
/// write-lock probe which can block for hundreds of ms on network/virtual drives.
/// This is sufficient for UI-thread decisions (watcher events, metadata requests)
/// because the full check will still be performed on the worker thread before any
/// file I/O occurs.
pub fn is_file_unsafe_to_read_fast(path: &Path) -> bool {
    if is_incomplete_download(path) {
        return true;
    }

    if is_recently_modified(path) {
        return true;
    }

    false
}

/// Time window (in seconds) during which a file is considered "still being
/// written" after its last modification. Applies to media files (video/image)
/// which are the primary targets of thumbnail/metadata extraction.
///
/// 2 minutes — generous enough for torrent inter-piece gaps (30-90s on slow
/// connections). The cost is just a delayed thumbnail; the benefit is not
/// killing a multi-GB download.
const WRITE_GUARD_SECS: u64 = 120;

/// Returns `true` if the file is a media file (video or image) AND was modified
/// within [`WRITE_GUARD_SECS`] seconds. Guards against race conditions where the
/// write-lock probe momentarily succeeds between torrent piece writes.
fn is_recently_modified(path: &Path) -> bool {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e,
        None => return false,
    };

    let lower = ext.to_ascii_lowercase();
    if !crate::infrastructure::windows::file_type::is_video_extension(&lower)
        && !crate::infrastructure::windows::file_type::is_image_extension(&lower)
    {
        return false;
    }

    match std::fs::metadata(path).and_then(|m| m.modified()) {
        Ok(mtime) => mtime
            .elapsed()
            .map_or(false, |elapsed| elapsed.as_secs() < WRITE_GUARD_SECS),
        Err(_) => false,
    }
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
