use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use parking_lot::RwLock;

use crate::file_index;
use crate::indexing_progress::IndexingProgress;
use crate::index_db;
use crate::usn_journal;
use super::upsert_volume_index;

const INCREMENTAL_APPLY_RETRY_ATTEMPTS: usize = 3;
const INCREMENTAL_APPLY_RETRY_SLEEP: std::time::Duration = std::time::Duration::from_millis(35);
/// Bounded fallback timeout for the write lock after try_write retries are
/// exhausted.  Prevents unbounded blocking that would starve concurrent
/// readers (search queries) via parking_lot's write-preferring fairness.
const INCREMENTAL_WRITE_FALLBACK_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(60);
const INCREMENTAL_CONTENTION_LOG_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

struct PendingPersistSnapshot {
    drive_letter: char,
    journal_id: u64,
    last_usn: i64,
    files_indexed: usize,
    additions: HashSet<u64>,
    removals: HashSet<u64>,
    addition_rows: Vec<(u64, String, u64, bool, bool, Vec<u64>)>,
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
    indexing_progress: Arc<IndexingProgress>,
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
            indexing_progress.set_error(drive_letter, 0, "open_volume");
            eprintln!("[USN] Failed to open volume {}:\\: {}", drive_letter, e);
            return;
        }
    };

    // Query USN Journal.
    let journal_info = match usn_journal::query_usn_journal(volume_handle) {
        Ok(info) => info,
        Err(e) => {
            indexing_progress.set_error(drive_letter, 0, "query_journal");
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
        if state.journal_id == journal_info.journal_id
            && state.has_hardlink_parent_data
            && state.has_reparse_point_data
        {
            indexing_progress.update(
                drive_letter,
                "scanning",
                0,
                "loading_cache",
                Some(0),
                Some(state.files_indexed),
            );
            // Stream records from DB directly into arena (no intermediate Vec<String>).
            if let Some(count) = db.load_into_index(&mut index, |loaded_count| {
                indexing_progress.update(
                    drive_letter,
                    "scanning",
                    loaded_count as u64,
                    "loading_cache",
                    Some(loaded_count as u64),
                    Some(state.files_indexed),
                )
            }) {
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
                index.hardlink_data_complete = state.has_hardlink_parent_data;
                index.reparse_data_complete = state.has_reparse_point_data;

                // DB-loaded rows are already persisted. Keep only real USN catch-up
                // changes as pending for the next incremental sync.
                index.clear_pending();
                indexing_progress.update(
                    drive_letter,
                    "scanning",
                    index.records.len() as u64,
                    "catching_up",
                    Some(index.records.len() as u64),
                    None,
                );

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

                        // Rebuild reverse children index after DB load.
                        index.rebuild_children();
                        if !index.hardlink_parents.is_empty() {
                            eprintln!(
                                "[USN] {}:\\ {} hardlinked files with {} extra parent entries",
                                drive_letter,
                                index.hardlink_parents.len(),
                                index.hardlink_parents.values().map(|v| v.len()).sum::<usize>()
                            );
                        }

                        // Extract file sizes from MFT (second pass).
                        let indexed_files = index.records.len() as u64;
                        indexing_progress.update(
                            drive_letter,
                            "scanning",
                            indexed_files,
                            "loading_sizes",
                            Some(0),
                            Some(indexed_files),
                        );
                        match crate::mft_reader::read_file_sizes(
                            volume_handle,
                            &mut index,
                            |processed, total| {
                                indexing_progress.update(
                                    drive_letter,
                                    "scanning",
                                    indexed_files,
                                    "loading_sizes",
                                    Some(processed),
                                    Some(total),
                                )
                            },
                        ) {
                            Ok(count) => {
                                eprintln!(
                                    "[MFT-SIZE] {}:\\ Populated {} file sizes after DB load",
                                    drive_letter, count
                                );
                            }
                            Err(e) => {
                                eprintln!(
                                    "[MFT-SIZE] {}:\\ Size extraction failed (non-fatal): {}",
                                    drive_letter,
                                    crate::redact_paths(&e)
                                );
                            }
                        }

                        need_full_scan = false;
                    }
                    Err(e) => {
                        indexing_progress.set_error(
                            drive_letter,
                            index.records.len() as u64,
                            "catching_up",
                        );
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
        } else if state.journal_id == journal_info.journal_id {
            eprintln!(
                "[USN] {}:\\ Cached index predates hardlink/reparse persistence; forcing one full MFT re-scan",
                drive_letter
            );
            need_full_scan = true;
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
        indexing_progress.set_scanning(drive_letter, index.records.len() as u64, "scanning_mft");
        eprintln!("[USN] {}:\\ Starting full MFT enumeration...", drive_letter);
        let start = std::time::Instant::now();

        match usn_journal::enumerate_all_files(
            volume_handle,
            &journal_info,
            &mut index,
            |count| {
                indexing_progress.update(
                    drive_letter,
                    "scanning",
                    count,
                    "scanning_mft",
                    Some(count),
                    None,
                )
            },
        ) {
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
                index.hardlink_data_complete = true;
                index.reparse_data_complete = true;
                let (arena_used, arena_cap, map_est) = index.memory_usage();
                eprintln!(
                    "[USN] {}:\\ Arena compacted: {:.1} MB -> {:.1} MB, map ~{:.1} MB, total ~{:.1} MB",
                    drive_letter,
                    arena_before as f64 / 1_048_576.0,
                    arena_used as f64 / 1_048_576.0,
                    map_est as f64 / 1_048_576.0,
                    (arena_cap + map_est) as f64 / 1_048_576.0
                );
                if !index.hardlink_parents.is_empty() {
                    eprintln!(
                        "[USN] {}:\\ {} hardlinked files with {} extra parent entries",
                        drive_letter,
                        index.hardlink_parents.len(),
                        index.hardlink_parents.values().map(|v| v.len()).sum::<usize>()
                    );
                }

                // Extract file sizes from MFT (second pass after enumeration).
                let indexed_files = index.records.len() as u64;
                indexing_progress.update(
                    drive_letter,
                    "scanning",
                    indexed_files,
                    "loading_sizes",
                    Some(0),
                    Some(indexed_files),
                );
                match crate::mft_reader::read_file_sizes(
                    volume_handle,
                    &mut index,
                    |processed, total| {
                        indexing_progress.update(
                            drive_letter,
                            "scanning",
                            indexed_files,
                            "loading_sizes",
                            Some(processed),
                            Some(total),
                        )
                    },
                ) {
                    Ok(count) => {
                        eprintln!(
                            "[MFT-SIZE] {}:\\ Populated {} file sizes after full scan",
                            drive_letter, count
                        );
                    }
                    Err(e) => {
                        eprintln!(
                            "[MFT-SIZE] {}:\\ Size extraction failed (non-fatal): {}",
                            drive_letter,
                            crate::redact_paths(&e)
                        );
                    }
                }
            }
            Err(e) => {
                indexing_progress.set_error(drive_letter, index.records.len() as u64, "scanning_mft");
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
        let persist_total = index.records.len() as u64;
        indexing_progress.update(
            drive_letter,
            "scanning",
            persist_total,
            "persisting",
            Some(0),
            Some(persist_total),
        );
        if let Err(e) = db.save_volume(&index, |inserted, total| {
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
    indexing_progress.clear(drive_letter);

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
                    // Bounded fallback: try_write_for avoids unbounded
                    // blocking that would starve search readers via
                    // parking_lot's write-preferring fairness policy.
                    // If the lock still can't be acquired, skip this
                    // cycle — current_usn stays unchanged so the next
                    // iteration re-reads from the same USN position
                    // (no data loss, at most ~2 s extra staleness).
                    match indices.try_write_for(INCREMENTAL_WRITE_FALLBACK_TIMEOUT) {
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
                            }
                        }
                        None => {
                            contention_skipped_cycles += 1;
                        }
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

        // 2b) Refresh file sizes for FRNs that had data changes.
        // Drain pending_size_refresh under a short lock, then read MFT records
        // without holding the lock, and finally apply sizes under a second lock.
        {
            let pending_frns: Vec<u64> = {
                match indices.try_write() {
                    Some(mut lock) => {
                        if let Some(vol) = lock.iter_mut().find(|v| v.drive_letter == drive_letter) {
                            if vol.sizes_loaded && !vol.pending_size_refresh.is_empty() {
                                std::mem::take(&mut vol.pending_size_refresh)
                                    .into_iter()
                                    .collect()
                            } else {
                                Vec::new()
                            }
                        } else {
                            Vec::new()
                        }
                    }
                    None => Vec::new(),
                }
            };

            if !pending_frns.is_empty() {
                // Read sizes without holding any lock (I/O phase).
                let geometry = crate::mft_reader::query_mft_geometry_pub(volume_handle);
                if let Ok(record_size) = geometry {
                    let mut size_updates: Vec<(u64, u64)> =
                        Vec::with_capacity(pending_frns.len());
                    for &frn in &pending_frns {
                        if let Some(size) =
                            crate::mft_reader::read_single_file_size(volume_handle, frn, record_size)
                        {
                            size_updates.push((frn, size));
                        }
                    }

                    // Apply sizes under lock.
                    if !size_updates.is_empty() {
                        if let Some(mut lock) = indices.try_write_for(INCREMENTAL_WRITE_FALLBACK_TIMEOUT) {
                            if let Some(vol) =
                                lock.iter_mut().find(|v| v.drive_letter == drive_letter)
                            {
                                for (frn, size) in &size_updates {
                                    if let Some(rec) = vol.records.get_mut(frn) {
                                        rec.size = *size;
                                    }
                                }
                            }
                        }
                    }
                }
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
                                    vol_index.reparse_points.contains(frn),
                                    vol_index
                                        .hardlink_parents
                                        .get(frn)
                                        .cloned()
                                        .unwrap_or_default(),
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
                    true,
                    true,
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

            // SEC: Prune stale dir_modified_at entries to prevent unbounded memory growth.
            // 10 minutes is generous enough to cover any realistic CheckPathsModified threshold.
            {
                let mut indices_lock = indices.write();
                if let Some(vol) = indices_lock.iter_mut().find(|v| v.drive_letter == drive_letter) {
                    vol.prune_old_modifications(std::time::Duration::from_secs(600));
                }
            }
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
