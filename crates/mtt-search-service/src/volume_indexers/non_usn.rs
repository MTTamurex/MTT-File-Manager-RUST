use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Storage::FileSystem::{
    FindCloseChangeNotification, FindFirstChangeNotificationW, FindNextChangeNotification,
    FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_CREATION, FILE_NOTIFY_CHANGE_DIR_NAME,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
};
use windows::Win32::System::Threading::WaitForSingleObject;

use super::wait_for_shutdown_or_timeout;
use crate::file_index;
use crate::fs_walker;
use crate::index_db;
use crate::indexing_progress::IndexingProgress;
use crate::volume_indices::{self, SharedVolumeIndices, VolumeIndexHandle};

const NON_USN_WAIT_STEP: std::time::Duration = std::time::Duration::from_millis(500);

#[derive(Clone, Copy)]
struct NonUsnScanCadence {
    periodic_interval: std::time::Duration,
    min_trigger_gap: std::time::Duration,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NonUsnWaitResult {
    Shutdown,
    Triggered,
    Timeout,
}

pub(crate) fn index_non_ntfs_volume(
    drive_letter: char,
    file_system: String,
    indices: SharedVolumeIndices,
    indexing_progress: Arc<IndexingProgress>,
    db: Arc<index_db::IndexDb>,
    shutdown: Arc<AtomicBool>,
) {
    let persisted_record_estimate = db
        .load_volume_state(drive_letter)
        .map(|state| state.files_indexed.min(usize::MAX as u64) as usize);

    let cadence = non_usn_scan_cadence(&file_system);
    let fs_lower = file_system.to_ascii_lowercase();
    let change_monitor = if is_fat_family_fs(&fs_lower) {
        None
    } else {
        NonUsnChangeMonitor::new(drive_letter)
    };

    eprintln!(
        "[SCAN] Starting fallback indexer for {}:\\ (filesystem: {}, periodic={}s, trigger-gap={}s, change-monitor={})",
        drive_letter,
        file_system,
        cadence.periodic_interval.as_secs(),
        cadence.min_trigger_gap.as_secs(),
        if change_monitor.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );

    // Fast startup path: reuse persisted snapshot while a fresh scan runs.
    let mut handle: Option<VolumeIndexHandle> = None;
    let mut cached_index = persisted_record_estimate
        .map(|estimate| file_index::VolumeIndex::with_estimated_records(drive_letter, estimate))
        .unwrap_or_else(|| file_index::VolumeIndex::empty(drive_letter));
    if let Ok(Some(cached_count)) = db.load_into_index(&mut cached_index, |_| {}) {
        cached_index.shrink_to_fit();
        cached_index.journal_id = 0;
        cached_index.last_usn = 0;
        cached_index.state = file_index::IndexState::Ready;

        handle = Some(volume_indices::upsert(&indices, cached_index));
        eprintln!(
            "[SCAN] {}:\\ Loaded {} cached records for fallback index",
            drive_letter, cached_count
        );
    }

    // Adaptive backoff: if consecutive scans show no change in record count,
    // increase the wait interval (up to 3× the base) to reduce I/O on idle volumes.
    // Reset to base interval as soon as a change is detected.
    // Note: record count alone can miss renames (delete+create = same count),
    // so we keep the multiplier conservative. Volumes with a change_monitor
    // (ReadDirectoryChangesW) are safe because the monitor wakes the loop
    // immediately on real changes regardless of backoff.
    let max_interval = cadence.periodic_interval * 3;
    let mut current_interval = cadence.periodic_interval;
    let mut prev_record_count: Option<usize> = persisted_record_estimate;

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let mut scanned_index = prev_record_count
            .map(|estimate| file_index::VolumeIndex::with_estimated_records(drive_letter, estimate))
            .unwrap_or_else(|| file_index::VolumeIndex::empty(drive_letter));
        scanned_index.state = file_index::IndexState::Scanning;
        indexing_progress.set_scanning(drive_letter, 0, "filesystem_scan");

        match fs_walker::scan_volume(drive_letter, &mut scanned_index, &shutdown, |count| {
            indexing_progress.update(
                drive_letter,
                "scanning",
                count,
                "filesystem_scan",
                Some(count),
                None,
            )
        }) {
            Ok(stats) => {
                scanned_index.journal_id = 0;
                scanned_index.last_usn = 0;
                scanned_index.state = file_index::IndexState::Ready;

                let persist_total = scanned_index.records.len() as u64;
                indexing_progress.update(
                    drive_letter,
                    "scanning",
                    persist_total,
                    "persisting",
                    Some(0),
                    Some(persist_total),
                );
                if let Err(e) = db.save_volume(&scanned_index, |inserted, total| {
                    indexing_progress.update(
                        drive_letter,
                        "scanning",
                        total,
                        "persisting",
                        Some(inserted),
                        Some(total),
                    )
                }) {
                    eprintln!(
                        "[SCAN] {}:\\ Failed to save fallback index: {}",
                        drive_letter,
                        crate::redact_paths(&e.to_string())
                    );
                }
                scanned_index.clear_pending();
                // SEC: Prune stale dir_modified_at entries.
                scanned_index.prune_old_modifications(std::time::Duration::from_secs(600));
                scanned_index.shrink_to_fit();

                let records = stats.records_indexed;
                handle = Some(volume_indices::upsert(&indices, scanned_index));
                indexing_progress.clear(drive_letter);

                // Adaptive backoff based on whether record count changed.
                let changed = prev_record_count.map_or(true, |prev| prev != records);
                prev_record_count = Some(records);

                if changed {
                    current_interval = cadence.periodic_interval;
                } else {
                    // Double the interval, capped at max_interval.
                    current_interval = (current_interval * 2).min(max_interval);
                }

                eprintln!(
                    "[SCAN] {}:\\ Indexed {} records ({} directories, {} read errors) in {:.2}s (next in {}s)",
                    drive_letter,
                    records,
                    stats.directories_scanned,
                    stats.errors,
                    stats.elapsed.as_secs_f64(),
                    current_interval.as_secs()
                );
            }
            Err(e) => {
                indexing_progress.set_error(
                    drive_letter,
                    scanned_index.records.len() as u64,
                    "filesystem_scan",
                );
                eprintln!(
                    "[SCAN] {}:\\ Full scan failed: {}",
                    drive_letter,
                    crate::redact_paths(&e)
                );

                if let Some(h) = handle.as_ref() {
                    let mut existing = h.write();
                    if existing.records.is_empty() {
                        existing.state = file_index::IndexState::Error(e.clone());
                    }
                }
            }
        }

        let wait_result = if let Some(monitor) = &change_monitor {
            monitor.wait_for_change_or_timeout(&shutdown, current_interval)
        } else if wait_for_shutdown_or_timeout(&shutdown, current_interval) {
            NonUsnWaitResult::Shutdown
        } else {
            NonUsnWaitResult::Timeout
        };

        match wait_result {
            NonUsnWaitResult::Shutdown => break,
            NonUsnWaitResult::Timeout => {}
            NonUsnWaitResult::Triggered => {
                // External change detected — reset backoff to base interval.
                current_interval = cadence.periodic_interval;
                // Guardrail against scan thrash on chatty filesystems.
                if wait_for_shutdown_or_timeout(&shutdown, cadence.min_trigger_gap) {
                    break;
                }
            }
        }
    }

    eprintln!("[SCAN] {}:\\ Fallback indexer stopped", drive_letter);
}

fn non_usn_scan_cadence(file_system: &str) -> NonUsnScanCadence {
    let fs = file_system.to_ascii_lowercase();
    if is_virtual_or_fuse_fs(&fs) {
        NonUsnScanCadence {
            periodic_interval: std::time::Duration::from_secs(30),
            min_trigger_gap: std::time::Duration::from_secs(3),
        }
    } else if is_fat_family_fs(&fs) {
        NonUsnScanCadence {
            periodic_interval: std::time::Duration::from_secs(120),
            min_trigger_gap: std::time::Duration::from_secs(15),
        }
    } else {
        NonUsnScanCadence {
            periodic_interval: std::time::Duration::from_secs(90),
            min_trigger_gap: std::time::Duration::from_secs(10),
        }
    }
}

fn is_virtual_or_fuse_fs(fs: &str) -> bool {
    fs.contains("cryptofs") || fs.contains("fuse") || fs.contains("dokan") || fs.contains("winfsp")
}

fn is_fat_family_fs(fs: &str) -> bool {
    matches!(fs, "exfat" | "fat32" | "fat" | "fat16" | "fat12")
}

struct NonUsnChangeMonitor {
    handle: HANDLE,
}

impl NonUsnChangeMonitor {
    fn new(drive_letter: char) -> Option<Self> {
        let root = format!("{}:\\", drive_letter);
        let wide_root: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();

        let notify_filter = FILE_NOTIFY_CHANGE_FILE_NAME
            | FILE_NOTIFY_CHANGE_DIR_NAME
            | FILE_NOTIFY_CHANGE_ATTRIBUTES
            | FILE_NOTIFY_CHANGE_SIZE
            | FILE_NOTIFY_CHANGE_LAST_WRITE
            | FILE_NOTIFY_CHANGE_CREATION;

        let handle = unsafe {
            FindFirstChangeNotificationW(PCWSTR(wide_root.as_ptr()), true, notify_filter)
        };

        match handle {
            Ok(h) if h != INVALID_HANDLE_VALUE => Some(Self { handle: h }),
            Ok(_) => {
                eprintln!(
                    "[SCAN] {}:\\ change monitor unavailable: invalid handle",
                    drive_letter
                );
                None
            }
            Err(e) => {
                eprintln!(
                    "[SCAN] {}:\\ change monitor unavailable: {}",
                    drive_letter,
                    crate::redact_paths(&e.to_string())
                );
                None
            }
        }
    }

    fn wait_for_change_or_timeout(
        &self,
        shutdown: &Arc<AtomicBool>,
        timeout: std::time::Duration,
    ) -> NonUsnWaitResult {
        let start = std::time::Instant::now();

        while start.elapsed() < timeout {
            if shutdown.load(Ordering::Relaxed) {
                return NonUsnWaitResult::Shutdown;
            }

            let remaining = timeout.saturating_sub(start.elapsed());
            let wait_for = NON_USN_WAIT_STEP.min(remaining);
            let wait_ms = wait_for.as_millis().min(u32::MAX as u128) as u32;
            let wait = unsafe { WaitForSingleObject(self.handle, wait_ms) };

            if wait == WAIT_OBJECT_0 {
                let rearmed = unsafe { FindNextChangeNotification(self.handle).is_ok() };
                if !rearmed {
                    eprintln!(
                        "[SCAN] non-USN change monitor rearm failed; falling back to periodic scans"
                    );
                    return NonUsnWaitResult::Timeout;
                }
                return NonUsnWaitResult::Triggered;
            }

            if wait == WAIT_TIMEOUT {
                continue;
            }

            eprintln!(
                "[SCAN] non-USN change monitor wait failed with status {}",
                wait.0
            );
            return NonUsnWaitResult::Timeout;
        }

        if shutdown.load(Ordering::Relaxed) {
            NonUsnWaitResult::Shutdown
        } else {
            NonUsnWaitResult::Timeout
        }
    }
}

impl Drop for NonUsnChangeMonitor {
    fn drop(&mut self) {
        unsafe {
            let _ = FindCloseChangeNotification(self.handle);
        }
    }
}
