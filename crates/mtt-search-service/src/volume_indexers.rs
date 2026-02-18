use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use crate::file_index;
use crate::fs_walker;
use crate::index_db;
use crate::usn_journal;

const INCREMENTAL_APPLY_RETRY_ATTEMPTS: usize = 3;
const INCREMENTAL_APPLY_RETRY_SLEEP: std::time::Duration = std::time::Duration::from_millis(35);
const INCREMENTAL_CONTENTION_LOG_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(30);

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
    let rescan_interval = non_usn_rescan_interval(&file_system);

    eprintln!(
        "[SCAN] Starting fallback indexer for {}:\\ (filesystem: {})",
        drive_letter, file_system
    );

    // Fast startup path: reuse persisted snapshot while a fresh scan runs.
    let mut cached_index = file_index::VolumeIndex::new(drive_letter);
    if let Some(cached_count) = db.load_into_index(&mut cached_index) {
        cached_index.names.shrink_to_fit();
        cached_index.journal_id = 0;
        cached_index.last_usn = 0;
        cached_index.state = file_index::IndexState::Ready;

        let mut indices_lock = indices.write().unwrap_or_else(|e| e.into_inner());
        upsert_volume_index(&mut indices_lock, cached_index);
        eprintln!(
            "[SCAN] {}:\\ Loaded {} cached records for fallback index",
            drive_letter, cached_count
        );
    }

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

                let records = stats.records_indexed;
                {
                    let mut indices_lock = indices.write().unwrap_or_else(|e| e.into_inner());
                    upsert_volume_index(&mut indices_lock, scanned_index);
                }

                eprintln!(
                    "[SCAN] {}:\\ Indexed {} records ({} directories, {} read errors) in {:.2}s",
                    drive_letter,
                    records,
                    stats.directories_scanned,
                    stats.errors,
                    stats.elapsed.as_secs_f64()
                );
            }
            Err(e) => {
                eprintln!(
                    "[SCAN] {}:\\ Full scan failed: {}",
                    drive_letter,
                    crate::redact_paths(&e)
                );

                let mut indices_lock = indices.write().unwrap_or_else(|poison| poison.into_inner());
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

        if wait_for_shutdown_or_timeout(&shutdown, rescan_interval) {
            break;
        }
    }

    eprintln!("[SCAN] {}:\\ Fallback indexer stopped", drive_letter);
}

fn non_usn_rescan_interval(file_system: &str) -> std::time::Duration {
    let fs = file_system.to_ascii_lowercase();
    if fs.contains("cryptofs")
        || fs.contains("fuse")
        || fs.contains("dokan")
        || fs.contains("winfsp")
    {
        // Virtual/encrypted mounts change frequently and usually have fewer entries.
        std::time::Duration::from_secs(30)
    } else {
        // Physical non-USN filesystems (e.g., exFAT/FAT32) keep a safer cadence.
        std::time::Duration::from_secs(120)
    }
}

fn upsert_volume_index(indices: &mut Vec<file_index::VolumeIndex>, new_index: file_index::VolumeIndex) {
    if let Some(existing) = indices
        .iter_mut()
        .find(|v| v.drive_letter == new_index.drive_letter)
    {
        *existing = new_index;
    } else {
        indices.push(new_index);
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
        "[USN] {}:\\ Journal ID: {}, Next USN: {}",
        drive_letter, journal_info.journal_id, journal_info.next_usn
    );

    let mut index = file_index::VolumeIndex::new(drive_letter);
    let need_full_scan;

    // Check if we can use cached data.
    if let Some(state) = cached_state {
        if state.journal_id == journal_info.journal_id {
            // Stream records from DB directly into arena (no intermediate Vec<String>).
            if let Some(count) = db.load_into_index(&mut index) {
                index.names.shrink_to_fit();
                let (arena_used, _arena_cap, map_est) = index.memory_usage();
                eprintln!(
                    "[USN] {}:\\ Loaded {} cached records, catching up from USN {}...",
                    drive_letter, count, state.last_usn
                );
                eprintln!(
                    "[USN] {}:\\ Memory after DB load: arena {:.1} MB, map ~{:.1} MB",
                    drive_letter,
                    arena_used as f64 / 1_048_576.0,
                    map_est as f64 / 1_048_576.0,
                );
                index.journal_id = state.journal_id;
                index.last_usn = state.last_usn;

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

        // Persist to database.
        if let Err(e) = db.save_volume(&index) {
            eprintln!(
                "[USN] {}:\\ Failed to save index: {}",
                drive_letter,
                crate::redact_paths(&e.to_string())
            );
        }
    }

    index.state = file_index::IndexState::Ready;
    let mut current_usn = index.last_usn;

    // Add to shared indices.
    {
        let mut indices_lock = indices.write().unwrap_or_else(|e| e.into_inner());
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
                        Ok(mut indices_lock) => {
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
                        Err(_) => {
                            attempt += 1;
                            if attempt < INCREMENTAL_APPLY_RETRY_ATTEMPTS {
                                contention_retries += 1;
                                std::thread::sleep(INCREMENTAL_APPLY_RETRY_SLEEP);
                            }
                        }
                    }
                }

                if !applied {
                    contention_skipped_cycles += 1;
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

        // 3) Persist every 5 minutes under read lock.
        if last_persist.elapsed() > std::time::Duration::from_secs(300) {
            let indices_lock = indices.read().unwrap_or_else(|e| e.into_inner());
            if let Some(vol_index) = indices_lock.iter().find(|v| v.drive_letter == drive_letter) {
                if let Err(e) = db.save_volume(vol_index) {
                    eprintln!(
                        "[USN] {}:\\ Persist error: {}",
                        drive_letter,
                        crate::redact_paths(&e.to_string())
                    );
                }
            }
            last_persist = std::time::Instant::now();
        }
    }

    if contention_retries > 0 || contention_applied_after_retry > 0 || contention_skipped_cycles > 0 {
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
