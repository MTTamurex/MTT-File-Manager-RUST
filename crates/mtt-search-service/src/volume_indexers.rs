use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use parking_lot::RwLock;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use std::collections::HashSet;
use windows::Win32::Storage::FileSystem::{
    FindCloseChangeNotification, FindFirstChangeNotificationW, FindNextChangeNotification,
    FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_CREATION, FILE_NOTIFY_CHANGE_DIR_NAME,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
};
use windows::Win32::System::Threading::WaitForSingleObject;

use crate::file_index;
use crate::fs_walker;
use crate::index_db;
use crate::usn_journal;

const INCREMENTAL_APPLY_RETRY_ATTEMPTS: usize = 3;
const INCREMENTAL_APPLY_RETRY_SLEEP: std::time::Duration = std::time::Duration::from_millis(35);
const INCREMENTAL_CONTENTION_LOG_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
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

pub(crate) fn wait_for_shutdown_or_timeout(
    shutdown: &Arc<AtomicBool>,
    timeout: std::time::Duration,
) -> bool {
    const STEP: std::time::Duration = std::time::Duration::from_millis(500);
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        if shutdown.load(Ordering::Relaxed) {
            return true;
        }

        let remaining = timeout.saturating_sub(start.elapsed());
        std::thread::sleep(STEP.min(remaining));
    }

    shutdown.load(Ordering::Relaxed)
}

pub(crate) fn index_non_ntfs_volume(
    drive_letter: char,
    file_system: String,
    indices: Arc<RwLock<Vec<file_index::VolumeIndex>>>,
    db: Arc<index_db::IndexDb>,
    shutdown: Arc<AtomicBool>,
) {
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
    let mut cached_index = file_index::VolumeIndex::new(drive_letter);
    if let Some(cached_count) = db.load_into_index(&mut cached_index) {
        cached_index.names.shrink_to_fit();
        cached_index.journal_id = 0;
        cached_index.last_usn = 0;
        cached_index.state = file_index::IndexState::Ready;

        let mut indices_lock = indices.write();
        upsert_volume_index(&mut indices_lock, cached_index);
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
    let mut prev_record_count: Option<usize> = None;

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let mut scanned_index = file_index::VolumeIndex::new(drive_letter);
        scanned_index.state = file_index::IndexState::Scanning;

        match fs_walker::scan_volume(drive_letter, &mut scanned_index, &shutdown) {
            Ok(stats) => {
                scanned_index.names.shrink_to_fit();
                scanned_index.journal_id = 0;
                scanned_index.last_usn = 0;
                scanned_index.state = file_index::IndexState::Ready;

                if let Err(e) = db.save_volume(&scanned_index) {
                    eprintln!(
                        "[SCAN] {}:\\ Failed to save fallback index: {}",
                        drive_letter,
                        crate::redact_paths(&e.to_string())
                    );
                }
                scanned_index.clear_pending();

                let records = stats.records_indexed;
                {
                    let mut indices_lock = indices.write();
                    upsert_volume_index(&mut indices_lock, scanned_index);
                }

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
                eprintln!(
                    "[SCAN] {}:\\ Full scan failed: {}",
                    drive_letter,
                    crate::redact_paths(&e)
                );

                let mut indices_lock = indices.write();
                if let Some(existing) = indices_lock
                    .iter_mut()
                    .find(|v| v.drive_letter == drive_letter)
                {
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
        // Virtual/encrypted mounts change frequently and usually have fewer entries.
        NonUsnScanCadence {
            periodic_interval: std::time::Duration::from_secs(30),
            min_trigger_gap: std::time::Duration::from_secs(3),
        }
    } else if is_fat_family_fs(&fs) {
        // Physical FAT-family filesystems (exFAT/FAT32/FAT) can be large and noisy.
        NonUsnScanCadence {
            periodic_interval: std::time::Duration::from_secs(120),
            min_trigger_gap: std::time::Duration::from_secs(15),
        }
    } else {
        // Conservative default for unknown non-USN filesystems.
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

struct PendingPersistSnapshot {
    drive_letter: char,
    journal_id: u64,
    last_usn: i64,
    files_indexed: usize,
    additions: HashSet<u64>,
    removals: HashSet<u64>,
    addition_rows: Vec<(u64, String, u64, bool)>,
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

fn upsert_volume_index(
    indices: &mut Vec<file_index::VolumeIndex>,
    new_index: file_index::VolumeIndex,
) {
    if let Some(existing) = indices
        .iter_mut()
        .find(|v| v.drive_letter == new_index.drive_letter)
    {
        *existing = new_index;
    } else {
        indices.push(new_index);
    }
}

fn restore_pending_snapshot(
    indices: &Arc<RwLock<Vec<file_index::VolumeIndex>>>,
    snapshot: PendingPersistSnapshot,
) {
    let mut indices_lock = indices.write();
    if let Some(vol_index) = indices_lock
        .iter_mut()
        .find(|v| v.drive_letter == snapshot.drive_letter)
    {
        vol_index.pending_additions.extend(snapshot.additions);
        vol_index.pending_removals.extend(snapshot.removals);
    }
}

pub(crate) fn index_volume(
    drive_letter: char,
    indices: Arc<RwLock<Vec<file_index::VolumeIndex>>>,
    db: Arc<index_db::IndexDb>,
    shutdown: Arc<AtomicBool>,
) {
    eprintln!("[USN] Starting indexing for volume {}:\\", drive_letter);

    // Try to load cached state from database.
    let cached_state = db.load_volume_state(drive_letter);

    // Open volume handle.
    let volume_handle = match usn_journal::open_volume(drive_letter) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[USN] Failed to open volume {}:\\: {}", drive_letter, e);
            return;
        }
    };

    // Query USN Journal.
    let journal_info = match usn_journal::query_usn_journal(volume_handle) {
        Ok(info) => info,
        Err(e) => {
            eprintln!(
                "[USN] Failed to query USN journal for {}:\\: {}",
                drive_letter, e
            );
            usn_journal::close_volume(volume_handle);
            return;
        }
    };

    eprintln!(
        "[USN] {}:\\ Journal ID: {}, First USN: {}, Next USN: {}",
        drive_letter, journal_info.journal_id, journal_info.first_usn, journal_info.next_usn
    );

    let mut index = file_index::VolumeIndex::new(drive_letter);
    let need_full_scan;

    // Check if we can use cached data.
    if let Some(state) = cached_state {
        if state.drive_letter != drive_letter {
            eprintln!(
                "[USN] Cache drive mismatch: expected {}:\\, got {}:\\; continuing with requested volume",
                drive_letter, state.drive_letter
            );
        }
        if state.journal_id == journal_info.journal_id {
            // Stream records from DB directly into arena (no intermediate Vec<String>).
            if let Some(count) = db.load_into_index(&mut index) {
                index.names.shrink_to_fit();
                let (arena_used, _arena_cap, map_est) = index.memory_usage();
                eprintln!(
                    "[USN] {}:\\ Loaded {} cached records (db reported {}), catching up from USN {}...",
                    drive_letter, count, state.files_indexed, state.last_usn
                );
                eprintln!(
                    "[USN] {}:\\ Memory after DB load: arena {:.1} MB, map ~{:.1} MB",
                    drive_letter,
                    arena_used as f64 / 1_048_576.0,
                    map_est as f64 / 1_048_576.0,
                );
                index.journal_id = state.journal_id;
                index.last_usn = state.last_usn;

                // DB-loaded rows are already persisted. Keep only real USN catch-up
                // changes as pending for the next incremental sync.
                index.clear_pending();

                // Catch up from last USN.
                match usn_journal::read_usn_changes(
                    volume_handle,
                    &journal_info,
                    index.last_usn,
                    &mut index,
                ) {
                    Ok(new_usn) => {
                        index.last_usn = new_usn;
                        eprintln!(
                            "[USN] {}:\\ Caught up to USN {}, {} total records",
                            drive_letter,
                            new_usn,
                            index.records.len()
                        );
                        need_full_scan = false;
                    }
                    Err(e) => {
                        eprintln!(
                            "[USN] {}:\\ Catch-up failed ({}), doing full scan",
                            drive_letter,
                            crate::redact_paths(&e)
                        );
                        index.clear();
                        need_full_scan = true;
                    }
                }
            } else {
                eprintln!(
                    "[USN] {}:\\ No cached records found, full scan needed",
                    drive_letter
                );
                need_full_scan = true;
            }
        } else {
            eprintln!(
                "[USN] {}:\\ Journal ID changed ({} -> {}), full re-scan needed",
                drive_letter, state.journal_id, journal_info.journal_id
            );
            need_full_scan = true;
        }
    } else {
        need_full_scan = true;
    }

    // Full MFT enumeration if needed.
    if need_full_scan {
        index.state = file_index::IndexState::Scanning;
        eprintln!("[USN] {}:\\ Starting full MFT enumeration...", drive_letter);
        let start = std::time::Instant::now();

        match usn_journal::enumerate_all_files(volume_handle, &journal_info, &mut index) {
            Ok(()) => {
                let elapsed = start.elapsed();
                index.journal_id = journal_info.journal_id;
                index.last_usn = journal_info.next_usn;
                eprintln!(
                    "[USN] {}:\\ Enumerated {} files in {:.2}s",
                    drive_letter,
                    index.records.len(),
                    elapsed.as_secs_f64()
                );

                // Compact arena: eliminate dead space from duplicate MFT names.
                let arena_before = index.names.len();
                index.compact_arena();
                let (arena_used, arena_cap, map_est) = index.memory_usage();
                eprintln!(
                    "[USN] {}:\\ Arena compacted: {:.1} MB -> {:.1} MB, map ~{:.1} MB, total ~{:.1} MB",
                    drive_letter,
                    arena_before as f64 / 1_048_576.0,
                    arena_used as f64 / 1_048_576.0,
                    map_est as f64 / 1_048_576.0,
                    (arena_cap + map_est) as f64 / 1_048_576.0
                );
            }
            Err(e) => {
                eprintln!(
                    "[USN] {}:\\ Enumeration failed: {}",
                    drive_letter,
                    crate::redact_paths(&e)
                );
                index.state = file_index::IndexState::Error(e);
                usn_journal::close_volume(volume_handle);
                return;
            }
        }

        // Persist to database (full save — initial scan).
        if let Err(e) = db.save_volume(&index) {
            eprintln!(
                "[USN] {}:\\ Failed to save index: {}",
                drive_letter,
                crate::redact_paths(&e.to_string())
            );
        }
        // Reset change tracking so the incremental sync starts fresh.
        index.clear_pending();
    }

    index.state = file_index::IndexState::Ready;
    let mut current_usn = index.last_usn;

    // Add to shared indices.
    {
        let mut indices_lock = indices.write();
        upsert_volume_index(&mut indices_lock, index);
    }

    eprintln!(
        "[USN] {}:\\ Index ready, starting incremental updates",
        drive_letter
    );

    // Incremental update loop.
    let mut last_persist = std::time::Instant::now();
    let mut contention_retries = 0u64;
    let mut contention_applied_after_retry = 0u64;
    let mut contention_skipped_cycles = 0u64;
    let mut last_contention_log = std::time::Instant::now();

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        std::thread::sleep(std::time::Duration::from_secs(2));

        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        // 1) Read raw USN buffer with no lock held.
        match usn_journal::read_usn_buffer(volume_handle, &journal_info, current_usn) {
            Ok(Some((buffer, bytes_returned, new_usn))) => {
                // 2) Apply deltas under a short bounded try_write retry window
                // to avoid read-starvation while reducing staleness under contention.
                let mut applied = false;
                let mut attempt = 0usize;
                while attempt < INCREMENTAL_APPLY_RETRY_ATTEMPTS {
                    match indices.try_write() {
                        Some(mut indices_lock) => {
                            if let Some(vol_index) = indices_lock
                                .iter_mut()
                                .find(|v| v.drive_letter == drive_letter)
                            {
                                let mut dummy_count = 0;
                                usn_journal::parse_usn_records(
                                    &buffer[8..bytes_returned as usize],
                                    vol_index,
                                    &mut dummy_count,
                                    true,
                                );
                                vol_index.last_usn = new_usn;
                                current_usn = new_usn;
                                applied = true;
                                if attempt > 0 {
                                    contention_applied_after_retry += 1;
                                }
                            }
                            break;
                        }
                        None => {
                            attempt += 1;
                            if attempt < INCREMENTAL_APPLY_RETRY_ATTEMPTS {
                                contention_retries += 1;
                                std::thread::sleep(INCREMENTAL_APPLY_RETRY_SLEEP);
                            }
                        }
                    }
                }

                if !applied {
                    // Fallback: blocking write to avoid silently dropping USN updates.
                    // With parking_lot this is safe (no poisoning) and bounded by reader drain time.
                    let mut indices_lock = indices.write();
                    if let Some(vol_index) = indices_lock
                        .iter_mut()
                        .find(|v| v.drive_letter == drive_letter)
                    {
                        let mut dummy_count = 0;
                        usn_journal::parse_usn_records(
                            &buffer[8..bytes_returned as usize],
                            vol_index,
                            &mut dummy_count,
                            true,
                        );
                        vol_index.last_usn = new_usn;
                        current_usn = new_usn;
                        applied = true;
                    }
                    if !applied {
                        contention_skipped_cycles += 1;
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!(
                    "[USN] {}:\\ Incremental read error: {}",
                    drive_letter,
                    crate::redact_paths(&e)
                );
            }
        }

        if last_contention_log.elapsed() >= INCREMENTAL_CONTENTION_LOG_INTERVAL {
            if contention_retries > 0
                || contention_applied_after_retry > 0
                || contention_skipped_cycles > 0
            {
                eprintln!(
                    "[USN] {}:\\ Incremental lock contention: retries={}, applied_after_retry={}, skipped_cycles={}",
                    drive_letter,
                    contention_retries,
                    contention_applied_after_retry,
                    contention_skipped_cycles
                );
                contention_retries = 0;
                contention_applied_after_retry = 0;
                contention_skipped_cycles = 0;
            }
            last_contention_log = std::time::Instant::now();
        }

        // 3) Persist every 5 minutes — incremental sync only (not full rebuild).
        if last_persist.elapsed() > std::time::Duration::from_secs(300) {
            let pending_snapshot = {
                let mut indices_lock = indices.write();
                indices_lock
                    .iter_mut()
                    .find(|v| v.drive_letter == drive_letter)
                    .map(|vol_index| {
                        let additions = std::mem::take(&mut vol_index.pending_additions);
                        let removals = std::mem::take(&mut vol_index.pending_removals);
                        let addition_rows = additions
                            .iter()
                            .filter_map(|frn| {
                                let record = vol_index.records.get(frn)?;
                                Some((
                                    *frn,
                                    vol_index.names.get(record.name_ref()).to_string(),
                                    record.parent_ref,
                                    record.is_dir,
                                ))
                            })
                            .collect();

                        PendingPersistSnapshot {
                            drive_letter: vol_index.drive_letter,
                            journal_id: vol_index.journal_id,
                            last_usn: vol_index.last_usn,
                            files_indexed: vol_index.records.len(),
                            additions,
                            removals,
                            addition_rows,
                        }
                    })
            };

            if let Some(snapshot) = pending_snapshot {
                if let Err(e) = db.save_volume_state_snapshot(
                    snapshot.drive_letter,
                    snapshot.journal_id,
                    snapshot.last_usn,
                    snapshot.files_indexed,
                ) {
                    eprintln!(
                        "[USN] {}:\\ Volume state persist error: {}",
                        drive_letter,
                        crate::redact_paths(&e.to_string())
                    );
                }

                if !snapshot.additions.is_empty() || !snapshot.removals.is_empty() {
                    if let Err(e) = db.sync_fts_incremental_snapshot(
                        snapshot.drive_letter,
                        &snapshot.addition_rows,
                        &snapshot.removals,
                    ) {
                        eprintln!(
                            "[USN] {}:\\ Incremental sync error (will retry): {}",
                            drive_letter,
                            crate::redact_paths(&e.to_string())
                        );
                        restore_pending_snapshot(&indices, snapshot);
                    }
                }
            }
            last_persist = std::time::Instant::now();
        }
    }

    if contention_retries > 0 || contention_applied_after_retry > 0 || contention_skipped_cycles > 0
    {
        eprintln!(
            "[USN] {}:\\ Final incremental contention stats: retries={}, applied_after_retry={}, skipped_cycles={}",
            drive_letter,
            contention_retries,
            contention_applied_after_retry,
            contention_skipped_cycles
        );
    }

    usn_journal::close_volume(volume_handle);
    eprintln!("[USN] {}:\\ Indexer stopped", drive_letter);
}
