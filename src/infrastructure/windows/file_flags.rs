use parking_lot::Mutex;
use std::fs::File;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
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
        .as_os_str()
        .encode_wide()
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
        .as_os_str()
        .encode_wide()
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

/// UI-thread fast guard window.
///
/// Keeps short-term protection right after MODIFY events, while avoiding long
/// post-download delays from fixed multi-minute cooldowns.
const FAST_RECENT_GUARD_SECS: u64 = 5;

/// Reduced stability window when the watcher has observed live write events.
///
/// The file system watcher (notify/ReadDirectoryChangesW) fires a Modify event
/// for every block qBittorrent (or any other writer) commits.  When those events
/// are the baseline, 3 s of silence reliably signals that writing has stopped.
const STABLE_AFTER_ACTIVITY_SECS: Duration = Duration::from_secs(3);

const STABILITY_STATE_CAP: usize = 8192;
const STABILITY_STATE_TTL: Duration = Duration::from_secs(15 * 60);
const WRITE_ACTIVITY_STATE_CAP: usize = 8192;
const WRITE_ACTIVITY_STATE_TTL: Duration = Duration::from_secs(2 * 60);
const COMPLETED_WRITE_STATE_CAP: usize = 2048;
const COMPLETED_WRITE_STATE_TTL: Duration = Duration::from_secs(5 * 60);

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

#[derive(Debug, Clone, Copy)]
struct CompletedWriteState {
    completed_at_system: SystemTime,
    recorded_at: Instant,
}

static FILE_STABILITY_CACHE: OnceLock<
    Mutex<std::collections::HashMap<PathBuf, FileStabilityState>>,
> = OnceLock::new();
static FILE_WRITE_ACTIVITY_CACHE: OnceLock<Mutex<std::collections::HashMap<PathBuf, Instant>>> =
    OnceLock::new();
static FILE_COMPLETED_WRITE_CACHE: OnceLock<
    Mutex<std::collections::HashMap<PathBuf, CompletedWriteState>>,
> = OnceLock::new();

fn get_file_stability_cache(
) -> &'static Mutex<std::collections::HashMap<PathBuf, FileStabilityState>> {
    FILE_STABILITY_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn get_file_write_activity_cache() -> &'static Mutex<std::collections::HashMap<PathBuf, Instant>> {
    FILE_WRITE_ACTIVITY_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn get_file_completed_write_cache(
) -> &'static Mutex<std::collections::HashMap<PathBuf, CompletedWriteState>> {
    FILE_COMPLETED_WRITE_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn is_path_or_descendant(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

fn remove_cached_paths_under_roots<T>(
    cache: &mut std::collections::HashMap<PathBuf, T>,
    roots: &[PathBuf],
) {
    cache.retain(|path, _| {
        !roots
            .iter()
            .any(|root| is_path_or_descendant(path, root.as_path()))
    });
}

fn remember_completed_write_roots(paths: &[PathBuf]) {
    if paths.is_empty() {
        return;
    }

    let now = Instant::now();
    let completed_at_system = SystemTime::now();
    let mut cache = get_file_completed_write_cache().lock();

    if cache.len() > COMPLETED_WRITE_STATE_CAP {
        cache.retain(|_, state| now.duration_since(state.recorded_at) <= COMPLETED_WRITE_STATE_TTL);
    }

    for path in paths {
        cache.insert(
            path.clone(),
            CompletedWriteState {
                completed_at_system,
                recorded_at: now,
            },
        );
    }
}

fn completed_write_covers_modified_path(path: &Path, modified: SystemTime) -> bool {
    let now = Instant::now();
    let mut cache = get_file_completed_write_cache().lock();

    if cache.len() > COMPLETED_WRITE_STATE_CAP {
        cache.retain(|_, state| now.duration_since(state.recorded_at) <= COMPLETED_WRITE_STATE_TTL);
    }

    cache.iter().any(|(root, state)| {
        now.duration_since(state.recorded_at) <= COMPLETED_WRITE_STATE_TTL
            && is_path_or_descendant(path, root.as_path())
            && modified <= state.completed_at_system
    })
}

fn completed_write_covers_current_path(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| completed_write_covers_modified_path(path, modified))
}

pub fn mark_recent_write_activity(path: &Path) {
    mark_recent_write_activity_at(path, Instant::now());
}

pub fn mark_recent_write_activity_at(path: &Path, seen_at: Instant) {
    if !is_media_file(path) {
        return;
    }

    // After Shell reports a copy/move complete, notify can still deliver old
    // Create/Modify events. Ignore only events for files whose current mtime is
    // covered by that completed operation; newer writes must remain guarded.
    if completed_write_covers_current_path(path) {
        return;
    }

    let now = Instant::now();
    let mut cache = get_file_write_activity_cache().lock();

    if cache.len() > WRITE_ACTIVITY_STATE_CAP {
        cache.retain(|_, seen_at| now.duration_since(*seen_at) <= WRITE_ACTIVITY_STATE_TTL);
    }

    cache.insert(path.to_path_buf(), seen_at.min(now));
}

/// Remove a file/folder from the write-activity and stability caches so that
/// `classify_file_read_safety` no longer considers it "recently changing".
pub fn clear_write_activity_for_path(path: &Path) {
    clear_write_activity_for_paths(&[path.to_path_buf()]);
}

/// Batch variant of [`clear_write_activity_for_path`].
pub fn clear_write_activity_for_paths(paths: &[std::path::PathBuf]) {
    if paths.is_empty() {
        return;
    }

    {
        let mut act = get_file_write_activity_cache().lock();
        remove_cached_paths_under_roots(&mut act, paths);
    }
    // Also reset stability tracking so the stability window isn't ticking
    // from the original create-event baseline.
    {
        let mut stab = get_file_stability_cache().lock();
        remove_cached_paths_under_roots(&mut stab, paths);
    }
}

/// Clear write-activity state after Shell reports that a copy/move finished.
///
/// Files with mtimes at or before completion are fully written and safe to read
/// immediately. Remembering the completed root also prevents delayed notify
/// events from the just-finished copy from renewing the write cooldown.
pub fn clear_write_activity_after_completed_file_operation(paths: &[std::path::PathBuf]) {
    clear_write_activity_for_paths(paths);
    remember_completed_write_roots(paths);
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
        .as_os_str()
        .encode_wide()
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

fn cached_write_activity_baseline(path: &Path) -> Option<Instant> {
    if !is_media_file(path) {
        return None;
    }

    let now = Instant::now();
    let mut cache = get_file_write_activity_cache().lock();
    if cache.len() > WRITE_ACTIVITY_STATE_CAP {
        cache.retain(|_, seen_at| now.duration_since(*seen_at) <= WRITE_ACTIVITY_STATE_TTL);
    }

    match cache.get(path).copied() {
        Some(seen_at) if now.duration_since(seen_at) <= WRITE_ACTIVITY_STATE_TTL => Some(seen_at),
        Some(_) => {
            cache.remove(path);
            None
        }
        None => None,
    }
}

fn recent_write_activity_baseline(path: &Path) -> Option<Instant> {
    if !is_media_file(path) {
        return None;
    }

    let now = Instant::now();
    let event_baseline = {
        let mut cache = get_file_write_activity_cache().lock();
        if cache.len() > WRITE_ACTIVITY_STATE_CAP {
            cache.retain(|_, seen_at| now.duration_since(*seen_at) <= WRITE_ACTIVITY_STATE_TTL);
        }

        match cache.get(path).copied() {
            Some(seen_at) if now.duration_since(seen_at) <= WRITE_ACTIVITY_STATE_TTL => {
                Some(seen_at)
            }
            Some(_) => {
                cache.remove(path);
                None
            }
            None => None,
        }
    };

    event_baseline
}

fn has_recent_write_activity(path: &Path, max_age: Duration) -> bool {
    cached_write_activity_baseline(path)
        .is_some_and(|baseline| Instant::now().duration_since(baseline) < max_age)
}

fn read_stability_snapshot(path: &Path) -> Option<FileStabilitySnapshot> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    Some(FileStabilitySnapshot {
        len: metadata.len(),
        modified,
    })
}

/// Tracks per-path stability across classify calls.  Returns `true` once the
/// file has been unchanged (same `len` + `mtime`) for at least `required_duration`
/// since the last observed write activity.
/// stability duration.  This lets `classify_file_read_safety` use a shorter
/// window (3 s) when the write-activity baseline comes from a live watcher event.
fn is_stable_enough_with_duration(
    path: &Path,
    snapshot: FileStabilitySnapshot,
    recent_write_baseline: Option<Instant>,
    required_duration: Duration,
) -> bool {
    let now = Instant::now();

    let mut cache = get_file_stability_cache().lock();

    if cache.len() > STABILITY_STATE_CAP {
        cache.retain(|_, state| now.duration_since(state.last_seen) <= STABILITY_STATE_TTL);
    }

    let key = path.to_path_buf();
    let is_stable = match cache.get_mut(&key) {
        Some(state) => {
            if let Some(baseline) = recent_write_baseline {
                if baseline > state.stable_since {
                    state.stable_since = baseline;
                }
            }

            if state.last_len != snapshot.len || state.last_modified != snapshot.modified {
                state.last_len = snapshot.len;
                state.last_modified = snapshot.modified;
                state.stable_since = now;
                state.last_seen = now;
                false
            } else {
                state.last_seen = now;
                now.duration_since(state.stable_since) >= required_duration
            }
        }
        None => {
            let stable_since = recent_write_baseline.unwrap_or(now);
            cache.insert(
                key,
                FileStabilityState {
                    last_len: snapshot.len,
                    last_modified: snapshot.modified,
                    stable_since,
                    last_seen: now,
                },
            );
            now.duration_since(stable_since) >= required_duration
        }
    };

    is_stable
}

/// Classify whether a file is currently safe to read for expensive media probes
/// (thumbnail extraction, metadata via COM/MF).
///
/// Worker-thread usage only: may perform lock probe and short sleep for
/// stability verification on recently modified media files.
///
/// # Ordering rationale
///
/// The lock probe (`is_file_locked_for_write`) opens the file **without**
/// `FILE_SHARE_WRITE`.  While that handle is alive, any concurrent `WriteFile`
/// from the writing process (e.g. qBittorrent) receives `ERROR_SHARING_VIOLATION`
/// and may abort the transfer.  Therefore the probe must **never** run when the
/// watcher has already established that the file is actively being written.
///
/// New order:
///   1. Extension blacklist (zero cost, no handle).
///   2. Watcher-event activity check.
///      • If activity is present → pure mtime/size stability via `std::fs::metadata`
///        (always uses share-all flags; safe for active writers).  The stability
///        window is `STABLE_AFTER_ACTIVITY_SECS` (3 s).
///      • If no activity at all → the lock probe runs as a last-resort fallback to
///        catch external writers whose `Create` event we may have missed (e.g. the
///        file existed before the watcher started).  This path is narrow: any file
///        touched after the watcher started will be in the activity cache.
pub fn classify_file_read_safety(path: &Path) -> FileReadSafety {
    if is_incomplete_download(path) {
        return FileReadSafety::IncompleteDownload;
    }

    let recent_write_baseline = recent_write_activity_baseline(path);

    if recent_write_baseline.is_some() {
        // Watcher (or recent mtime) has observed write activity on this file.
        // Use only std::fs::metadata-based stability (share-all, never hostile to
        // active writers) — do NOT call is_file_locked_for_write here.
        let snapshot = match read_stability_snapshot(path) {
            Some(v) => v,
            None => return FileReadSafety::RecentlyChanging,
        };

        // Watcher activity is the only baseline here. Do not use mtime recency
        // as a write signal: copied media often has fresh mtimes after restart.
        if !is_stable_enough_with_duration(
            path,
            snapshot,
            recent_write_baseline,
            STABLE_AFTER_ACTIVITY_SECS,
        ) {
            return FileReadSafety::RecentlyChanging;
        }

        return FileReadSafety::Safe;
    }

    if completed_write_covers_current_path(path) {
        return FileReadSafety::Safe;
    }

    // No watcher activity observed for this path.  Run the lock probe as a
    // last-resort to catch writers whose Create/Modify events we missed (e.g.
    // file was being written before the watcher started watching this folder).
    // This path will NOT be hit for any active qBittorrent download because
    // qBittorrent always triggers at least a Create event when it first writes
    // to the file, which populates the activity cache.
    if is_file_locked_for_write(path) {
        return FileReadSafety::WriteLocked;
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
/// Only performs cheap checks (extension + cached watcher activity), skipping
/// filesystem metadata, the `CreateFileW` write-lock probe, and stability delay
/// which can block on network/virtual drives.
/// This is sufficient for UI-thread decisions (watcher events, metadata requests)
/// because the full check will still be performed on the worker thread before any
/// file I/O occurs.
pub fn is_file_unsafe_to_read_fast(path: &Path) -> bool {
    if is_incomplete_download(path) {
        return true;
    }

    if has_recent_write_activity(path, Duration::from_secs(FAST_RECENT_GUARD_SECS)) {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

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

    #[test]
    fn fast_read_safety_flags_incomplete_download_extensions() {
        assert!(is_file_unsafe_to_read_fast(Path::new(
            "video.mp4.crdownload"
        )));
    }

    #[test]
    fn fast_read_safety_uses_cached_write_activity() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("video.mp4");

        clear_write_activity_for_path(&path);
        assert!(!is_file_unsafe_to_read_fast(&path));

        mark_recent_write_activity(&path);
        assert!(is_file_unsafe_to_read_fast(&path));

        clear_write_activity_for_path(&path);
    }

    #[test]
    fn fast_read_safety_does_not_probe_recent_media_mtime() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("recent.mp4");
        let mut file = std::fs::File::create(&path).expect("create media file");
        file.write_all(b"not real media").expect("write media file");

        clear_write_activity_for_path(&path);

        assert!(!is_file_unsafe_to_read_fast(&path));
    }

    #[test]
    fn clear_write_activity_for_folder_clears_descendant_activity() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let root = dir.path().join("copied");
        let path = root.join("nested").join("video.mp4");

        mark_recent_write_activity(&path);
        assert!(is_file_unsafe_to_read_fast(&path));

        clear_write_activity_for_path(&root);
        assert!(!is_file_unsafe_to_read_fast(&path));
    }

    #[test]
    fn recent_mtime_without_watcher_event_does_not_block_media() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let root = dir.path().join("copied");
        std::fs::create_dir_all(&root).expect("create copied folder");
        let path = root.join("video.mp4");
        {
            let mut file = std::fs::File::create(&path).expect("create media file");
            file.write_all(b"not real media").expect("write media file");
        }

        assert_eq!(classify_file_read_safety(&path), FileReadSafety::Safe);
    }

    #[test]
    fn completed_copy_root_ignores_delayed_watcher_write_activity() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let root = dir.path().join("copied");
        std::fs::create_dir_all(&root).expect("create copied folder");
        let path = root.join("video.mp4");
        {
            let mut file = std::fs::File::create(&path).expect("create media file");
            file.write_all(b"not real media").expect("write media file");
        }

        clear_write_activity_after_completed_file_operation(&[root]);
        mark_recent_write_activity(&path);

        assert!(!is_file_unsafe_to_read_fast(&path));
        assert_eq!(classify_file_read_safety(&path), FileReadSafety::Safe);
    }

    #[test]
    fn completed_copy_root_records_post_completion_write_activity() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let root = dir.path().join("copied");
        std::fs::create_dir_all(&root).expect("create copied folder");
        let path = root.join("video.mp4");
        {
            let mut file = std::fs::File::create(&path).expect("create media file");
            file.write_all(b"not real media").expect("write media file");
        }

        clear_write_activity_after_completed_file_operation(&[root.clone()]);

        {
            let mut cache = get_file_completed_write_cache().lock();
            cache
                .get_mut(&root)
                .expect("completed root is remembered")
                .completed_at_system = SystemTime::UNIX_EPOCH;
        }

        {
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .expect("open media file");
            file.write_all(b"new bytes").expect("append media file");
        }

        mark_recent_write_activity(&path);

        assert!(is_file_unsafe_to_read_fast(&path));
        assert_eq!(
            classify_file_read_safety(&path),
            FileReadSafety::RecentlyChanging
        );

        clear_write_activity_for_path(&root);
        get_file_completed_write_cache().lock().remove(&root);
    }
}
