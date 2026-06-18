use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::file_index;
use crate::index_db;
use crate::indexing_progress::IndexingProgress;
use crate::usn_journal;
use crate::volume_indices::{self, SharedVolumeIndices, VolumeIndexHandle};

const INCREMENTAL_APPLY_RETRY_ATTEMPTS: usize = 3;
const INCREMENTAL_APPLY_RETRY_SLEEP: std::time::Duration = std::time::Duration::from_millis(35);
/// Bounded fallback timeout for the write lock after try_write retries are
/// exhausted.  Prevents unbounded blocking that would starve concurrent
/// readers (search queries) via parking_lot's write-preferring fairness.
const INCREMENTAL_WRITE_FALLBACK_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(60);
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

fn restore_pending_snapshot(handle: &VolumeIndexHandle, snapshot: PendingPersistSnapshot) {
    let mut vol_index = handle.write();
    vol_index.pending_additions.extend(snapshot.additions);
    vol_index.pending_removals.extend(snapshot.removals);
}

fn take_pending_snapshot(handle: &VolumeIndexHandle) -> PendingPersistSnapshot {
    let mut vol_index = handle.write();
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
}

fn persist_pending_snapshot(
    db: &index_db::IndexDb,
    handle: &VolumeIndexHandle,
    drive_letter: char,
    snapshot: PendingPersistSnapshot,
) -> bool {
    let had_structural_changes = !snapshot.additions.is_empty() || !snapshot.removals.is_empty();
    let skip_sqlite_data = crate::index_db::skip_sqlite_data_persistence();

    if skip_sqlite_data && !flush_binary_snapshot_if_dirty(handle, drive_letter) {
        eprintln!(
            "[USN] {}:\\ Deferring volume_state persist until binary snapshot is durable",
            drive_letter
        );
        restore_pending_snapshot(handle, snapshot);
        return false;
    }

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

    if had_structural_changes && !skip_sqlite_data {
        if let Err(e) = db.sync_records_incremental_snapshot(
            snapshot.drive_letter,
            &snapshot.addition_rows,
            &snapshot.removals,
        ) {
            eprintln!(
                "[USN] {}:\\ Incremental sync error (will retry): {}",
                drive_letter,
                crate::redact_paths(&e.to_string())
            );
            restore_pending_snapshot(handle, snapshot);
            return false;
        }
    }

    had_structural_changes
}

fn flush_binary_snapshot_if_dirty(handle: &VolumeIndexHandle, drive_letter: char) -> bool {
    let should_trim = {
        let mut vol = handle.write();
        if !vol.binary_dirty {
            return true;
        }
        if !vol.sizes_loaded {
            return false;
        }

        match crate::index_db::binary::save(&vol) {
            Ok(()) => {
                vol.binary_dirty = false;
                true
            }
            Err(e) => {
                eprintln!(
                    "[USN] {}:\\ Binary snapshot flush failed: {}",
                    drive_letter,
                    crate::redact_paths(&e)
                );
                false
            }
        }
    };

    if should_trim {
        crate::memory_trim::trim_working_set(&format!("{}:\\ binary snapshot flush", drive_letter));
    }
    should_trim
}

fn should_prefer_sqlite_over_binary(
    sqlite_state: &index_db::PersistedVolumeState,
    binary_state: &index_db::PersistedVolumeState,
    current_journal_id: u64,
) -> bool {
    let sqlite_matches_current = sqlite_state.journal_id == current_journal_id;
    let binary_matches_current = binary_state.journal_id == current_journal_id;
    let sqlite_is_fresher_same_journal = sqlite_state.journal_id == binary_state.journal_id
        && sqlite_state.last_usn > binary_state.last_usn;

    sqlite_is_fresher_same_journal || (sqlite_matches_current && !binary_matches_current)
}

pub(crate) fn index_volume(
    drive_letter: char,
    indices: SharedVolumeIndices,
    indexing_progress: Arc<IndexingProgress>,
    db: Arc<index_db::IndexDb>,
    shutdown: Arc<AtomicBool>,
) {
    eprintln!("[USN] Starting indexing for volume {}:\\", drive_letter);

    let mut index = file_index::VolumeIndex::empty(drive_letter);
    let sqlite_state = db.load_volume_state(drive_letter);

    // Try to load cached state — prefer binary file, fall back to SQLite.
    let binary_candidate = match crate::index_db::binary::load(drive_letter) {
        Ok(Some((bin_index, bin_state))) => Some((
            bin_index,
            crate::index_db::PersistedVolumeState {
                drive_letter,
                journal_id: bin_state.journal_id,
                last_usn: bin_state.last_usn,
                files_indexed: bin_state.files_indexed,
                has_hardlink_parent_data: bin_state.has_hardlink_parent_data,
                has_reparse_point_data: bin_state.has_reparse_point_data,
            },
        )),
        Ok(None) => None,
        Err(e) => {
            eprintln!(
                "[USN] {}:\\ Binary index load failed ({}), trying SQLite",
                drive_letter, e
            );
            None
        }
    };

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

    let (cached_state, loaded_from_binary) = match (binary_candidate, sqlite_state) {
        (Some((bin_index, bin_state)), Some(sqlite_state))
            if should_prefer_sqlite_over_binary(
                &sqlite_state,
                &bin_state,
                journal_info.journal_id,
            ) =>
        {
            eprintln!(
                "[USN] {}:\\ Discarding stale binary snapshot in favor of fresher SQLite metadata (bin_journal={}, bin_usn={}, db_journal={}, db_usn={})",
                drive_letter,
                bin_state.journal_id,
                bin_state.last_usn,
                sqlite_state.journal_id,
                sqlite_state.last_usn,
            );
            if let Err(e) = std::fs::remove_file(crate::index_db::binary::index_path(drive_letter))
            {
                if e.kind() != std::io::ErrorKind::NotFound {
                    eprintln!(
                        "[USN] {}:\\ Failed to remove stale binary snapshot: {}",
                        drive_letter,
                        crate::redact_paths(&e.to_string())
                    );
                }
            }
            drop(bin_index);
            (Some(sqlite_state), false)
        }
        (Some((bin_index, bin_state)), _) => {
            index = bin_index;
            (Some(bin_state), true)
        }
        (None, Some(sqlite_state)) => (Some(sqlite_state), false),
        (None, None) => (None, false),
    };

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
            // Load records from cache (binary already loaded or fall back to DB).
            let loaded_count = if loaded_from_binary {
                let count = index.records.len();
                eprintln!(
                    "[USN] {}:\\ Loaded {} records from binary cache, catching up from USN {}...",
                    drive_letter, count, state.last_usn
                );
                Some(count)
            } else {
                index = file_index::VolumeIndex::with_estimated_records(
                    drive_letter,
                    state.files_indexed.min(usize::MAX as u64) as usize,
                );
                indexing_progress.update(
                    drive_letter,
                    "scanning",
                    0,
                    "loading_cache",
                    Some(0),
                    Some(state.files_indexed),
                );
                match db.load_into_index(&mut index, |loaded| {
                    indexing_progress.update(
                        drive_letter,
                        "scanning",
                        loaded as u64,
                        "loading_cache",
                        Some(loaded as u64),
                        Some(state.files_indexed),
                    )
                }) {
                    Ok(loaded) => loaded,
                    Err(error) => {
                        eprintln!(
                            "[USN] {}:\\ SQLite cache load failed ({}), full scan needed",
                            drive_letter,
                            crate::redact_paths(&error)
                        );
                        None
                    }
                }
            };

            if let Some(count) = loaded_count {
                if !loaded_from_binary && count as u64 != state.files_indexed {
                    eprintln!(
                        "[USN] {}:\\ SQLite fallback snapshot is incomplete (loaded {} rows, volume_state says {}); forcing full scan",
                        drive_letter,
                        count,
                        state.files_indexed,
                    );
                    index.clear();
                    need_full_scan = true;
                } else {
                    index.shrink_to_fit();
                    let (arena_used, _arena_cap, records_est) = index.memory_usage();
                    if !loaded_from_binary {
                        eprintln!(
                            "[USN] {}:\\ Loaded {} cached records (db reported {}), catching up from USN {}...",
                            drive_letter, count, state.files_indexed, state.last_usn
                        );
                    }
                    eprintln!(
                        "[USN] {}:\\ Memory after cache load: arena {:.1} MB, records ~{:.1} MB",
                        drive_letter,
                        arena_used as f64 / 1_048_576.0,
                        records_est as f64 / 1_048_576.0,
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

                            // Rebuild reverse children index after cache load
                            // (binary load already does this, but USN catch-up may
                            // have added/moved entries).
                            index.rebuild_children();
                            if !index.hardlink_parents.is_empty() {
                                eprintln!(
                                    "[USN] {}:\\ {} hardlinked files with {} extra parent entries",
                                    drive_letter,
                                    index.hardlink_parents.len(),
                                    index
                                        .hardlink_parents
                                        .values()
                                        .map(|v| v.len())
                                        .sum::<usize>()
                                );
                            }

                            // File sizes are deferred to a background thread after
                            // the volume is marked Ready (Phase 1 optimisation).
                            // This lets search results appear within seconds instead
                            // of waiting minutes for per-file MFT IOCTL reads.

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
        eprintln!("[USN] {}:\\ Starting bulk MFT read...", drive_letter);
        let start = std::time::Instant::now();

        match crate::mft_reader::read_mft_bulk(volume_handle, drive_letter, |done, total| {
            indexing_progress.update(
                drive_letter,
                "scanning",
                done,
                "scanning_mft",
                Some(done),
                Some(total),
            )
        }) {
            Ok(mut new_index) => {
                let elapsed = start.elapsed();
                new_index.journal_id = journal_info.journal_id;
                new_index.last_usn = journal_info.next_usn;
                new_index.hardlink_data_complete = true;
                new_index.reparse_data_complete = true;
                new_index.sizes_loaded = true;
                eprintln!(
                    "[USN] {}:\\ Bulk MFT read: {} files in {:.2}s",
                    drive_letter,
                    new_index.records.len(),
                    elapsed.as_secs_f64()
                );

                // Compact arena only when it would reclaim meaningful dead
                // name bytes. Current MFT parsing selects one display name per
                // FRN, so most full scans have little/no dead arena space; an
                // unconditional compact would briefly allocate a second arena.
                const MIN_ARENA_COMPACTION_SAVINGS: usize = 8 * 1024 * 1024;
                let arena_before = new_index.names.len();
                let referenced_name_bytes = new_index.referenced_name_bytes();
                let dead_name_bytes = arena_before.saturating_sub(referenced_name_bytes);
                let arena_compacted = dead_name_bytes >= MIN_ARENA_COMPACTION_SAVINGS;
                if arena_compacted {
                    new_index.compact_arena();
                } else {
                    new_index.shrink_to_fit();
                }
                let (arena_used, arena_cap, records_est) = new_index.memory_usage();
                eprintln!(
                    "[USN] {}:\\ Arena {}: {:.1} MB -> {:.1} MB, dead {:.1} MB, records ~{:.1} MB, total ~{:.1} MB",
                    drive_letter,
                    if arena_compacted { "compacted" } else { "shrink-only" },
                    arena_before as f64 / 1_048_576.0,
                    arena_used as f64 / 1_048_576.0,
                    dead_name_bytes as f64 / 1_048_576.0,
                    records_est as f64 / 1_048_576.0,
                    (arena_cap + records_est) as f64 / 1_048_576.0
                );
                if !new_index.hardlink_parents.is_empty() {
                    eprintln!(
                        "[USN] {}:\\ {} hardlinked files with {} extra parent entries",
                        drive_letter,
                        new_index.hardlink_parents.len(),
                        new_index
                            .hardlink_parents
                            .values()
                            .map(|v| v.len())
                            .sum::<usize>()
                    );
                }
                index = new_index;
            }
            Err(e) => {
                indexing_progress.set_error(
                    drive_letter,
                    index.records.len() as u64,
                    "scanning_mft",
                );
                eprintln!(
                    "[USN] {}:\\ Bulk MFT read failed: {}",
                    drive_letter,
                    crate::redact_paths(&e)
                );
                index.state = file_index::IndexState::Error(e);
                usn_journal::close_volume(volume_handle);
                return;
            }
        }

        // Persist to binary format (fast — typically <1s).
        indexing_progress.update(
            drive_letter,
            "scanning",
            index.records.len() as u64,
            "persisting",
            None,
            None,
        );
        let binary_saved = match crate::index_db::binary::save(&index) {
            Ok(()) => {
                index.binary_dirty = false;
                true
            }
            Err(e) => {
                eprintln!(
                    "[USN] {}:\\ Binary save failed: {}",
                    drive_letter,
                    crate::redact_paths(&e)
                );
                false
            }
        };
        let skip_sqlite_data = crate::index_db::skip_sqlite_data_persistence();

        // Persist volume state (journal_id/last_usn) to SQLite so incremental
        // resume still works even when file_records are stored only in the
        // binary index. Do not advance SQLite metadata in skip mode unless the
        // matching binary snapshot was saved; otherwise a later SQLite fallback
        // could treat stale rows as current.
        if !skip_sqlite_data || binary_saved {
            if let Err(e) = db.save_volume_state_snapshot(
                drive_letter,
                index.journal_id,
                index.last_usn,
                index.records.len(),
                index.hardlink_data_complete,
                index.reparse_data_complete,
            ) {
                eprintln!(
                    "[USN] {}:\\ SQLite volume state snapshot failed: {}",
                    drive_letter,
                    crate::redact_paths(&e)
                );
            }
        }

        if skip_sqlite_data {
            // Binary index is the authoritative store for this NTFS volume.
            // Purge any stale SQLite file_records/hardlink_parents rows to
            // reclaim disk space. If the binary index is later corrupted, the
            // service will rebuild from a full scan.
            if binary_saved {
                db.purge_volume_data(drive_letter);
            } else {
                eprintln!(
                    "[USN] {}:\\ Keeping existing SQLite data because binary save failed",
                    drive_letter
                );
            }
        } else {
            // Keep SQLite as a true full-volume fallback snapshot. Without this,
            // losing the binary cache can resurrect a sparse incremental-only DB
            // view after restart, which breaks search and folder-size resolution
            // for most paths on the volume.
            indexing_progress.update(
                drive_letter,
                "scanning",
                index.records.len() as u64,
                "persisting_sqlite",
                None,
                None,
            );
            if let Err(e) = db.save_volume(&index, |inserted, total| {
                indexing_progress.update(
                    drive_letter,
                    "scanning",
                    inserted,
                    "persisting_sqlite",
                    Some(inserted),
                    Some(total),
                );
            }) {
                eprintln!(
                    "[USN] {}:\\ SQLite full snapshot save failed: {}",
                    drive_letter,
                    crate::redact_paths(&e)
                );
            }
        }

        // Reset change tracking so the incremental sync starts fresh.
        index.clear_pending();
    }

    index.shrink_to_fit();
    index.state = file_index::IndexState::Ready;
    let mut current_usn = index.last_usn;
    let sizes_already_loaded = index.sizes_loaded;

    // Register / update this volume's handle. From here on, all per-volume
    // mutations happen through `handle` directly — the outer registry lock is
    // not taken again, so concurrent writers/readers on *other* volumes are
    // never blocked by this thread.
    let handle: VolumeIndexHandle = volume_indices::upsert(&indices, index);
    indexing_progress.clear(drive_letter);
    if sizes_already_loaded {
        crate::memory_trim::trim_working_set(&format!("{}:\\ index ready", drive_letter));
        crate::memory_trim::trim_working_set_delayed(
            format!("{}:\\ index ready delayed", drive_letter),
            std::time::Duration::from_secs(10),
        );
        crate::memory_trim::trim_working_set_delayed(
            format!("{}:\\ index ready idle", drive_letter),
            std::time::Duration::from_secs(60),
        );
    }

    // Background file size extraction — only needed when loaded from a cache
    // that doesn't have sizes (old binary format or SQLite fallback).
    // After a bulk MFT read, sizes_loaded is already true.
    if !sizes_already_loaded {
        let bg_handle = handle.clone();
        let indexing_progress = indexing_progress.clone();
        std::thread::spawn(move || {
            // Open a dedicated volume handle for this background thread.
            let bg_volume = match usn_journal::open_volume(drive_letter) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!(
                        "[MFT-SIZE] {}:\\ Background size extraction: failed to open volume: {}",
                        drive_letter, e
                    );
                    return;
                }
            };

            eprintln!(
                "[MFT-SIZE] {}:\\ Background size extraction via sizes-only MFT read...",
                drive_letter,
            );
            let start = std::time::Instant::now();

            // Extract only size updates in a single sequential I/O pass. This
            // avoids building a second full VolumeIndex just to copy sizes.
            let bulk_result =
                crate::mft_reader::read_mft_sizes_bulk(bg_volume, drive_letter, |done, total| {
                    indexing_progress.update(
                        drive_letter,
                        "scanning",
                        done,
                        "loading_sizes",
                        Some(done),
                        Some(total),
                    );
                });

            usn_journal::close_volume(bg_volume);

            match bulk_result {
                Ok(size_updates) => {
                    // Apply sizes from the MFT pass to the live index.
                    let mut applied = 0u64;
                    let mut sizes_marked = false;
                    if let Some(mut vol) =
                        bg_handle.try_write_for(std::time::Duration::from_secs(10))
                    {
                        let mut changed = false;
                        for (frn, size) in &size_updates {
                            if *size > 0 {
                                if let Some(rec) = vol.records.get_mut(frn) {
                                    let new_size = rec.size.max(*size);
                                    if rec.size != new_size {
                                        rec.size = new_size;
                                        applied += 1;
                                        changed = true;
                                    }
                                }
                            }
                        }
                        if !vol.sizes_loaded {
                            changed = true;
                        }
                        vol.sizes_loaded = true;
                        if changed {
                            vol.binary_dirty = true;
                        }
                        sizes_marked = true;
                    }
                    if !sizes_marked {
                        loop {
                            if let Some(mut vol) =
                                bg_handle.try_write_for(std::time::Duration::from_millis(250))
                            {
                                let mut changed = false;
                                for (frn, size) in &size_updates {
                                    if *size > 0 {
                                        if let Some(rec) = vol.records.get_mut(frn) {
                                            let new_size = rec.size.max(*size);
                                            if rec.size != new_size {
                                                rec.size = new_size;
                                                applied += 1;
                                                changed = true;
                                            }
                                        }
                                    }
                                }
                                if !vol.sizes_loaded {
                                    changed = true;
                                }
                                vol.sizes_loaded = true;
                                if changed {
                                    vol.binary_dirty = true;
                                }
                                break;
                            }
                        }
                    }
                    let elapsed = start.elapsed();
                    eprintln!(
                        "[MFT-SIZE] {}:\\ Bulk size extraction complete: {} sizes updated in {:.2}s",
                        drive_letter, applied, elapsed.as_secs_f64()
                    );
                }
                Err(e) => {
                    eprintln!(
                        "[MFT-SIZE] {}:\\ Bulk size extraction failed: {}, keeping sizes unloaded",
                        drive_letter, e
                    );
                }
            }

            crate::memory_trim::trim_working_set(&format!(
                "{}:\\ background size extraction",
                drive_letter
            ));
            crate::memory_trim::trim_working_set_delayed(
                format!("{}:\\ background size extraction delayed", drive_letter),
                std::time::Duration::from_secs(10),
            );
            indexing_progress.clear(drive_letter);
        });
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
                    match handle.try_write() {
                        Some(mut vol_index) => {
                            let mut dummy_count = 0;
                            usn_journal::parse_usn_records(
                                &buffer[8..bytes_returned as usize],
                                &mut vol_index,
                                &mut dummy_count,
                                true,
                            );
                            vol_index.last_usn = new_usn;
                            current_usn = new_usn;
                            applied = true;
                            if attempt > 0 {
                                contention_applied_after_retry += 1;
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
                    match handle.try_write_for(INCREMENTAL_WRITE_FALLBACK_TIMEOUT) {
                        Some(mut vol_index) => {
                            let mut dummy_count = 0;
                            usn_journal::parse_usn_records(
                                &buffer[8..bytes_returned as usize],
                                &mut vol_index,
                                &mut dummy_count,
                                true,
                            );
                            vol_index.last_usn = new_usn;
                            current_usn = new_usn;
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
            let pending_frns: Vec<u64> = match handle.try_write() {
                Some(mut vol) => {
                    if vol.sizes_loaded && !vol.pending_size_refresh.is_empty() {
                        std::mem::take(&mut vol.pending_size_refresh)
                            .into_iter()
                            .collect()
                    } else {
                        Vec::new()
                    }
                }
                None => Vec::new(),
            };

            if !pending_frns.is_empty() {
                // Read sizes without holding any lock (I/O phase).
                let geometry = crate::mft_reader::query_mft_geometry_pub(volume_handle);
                if let Ok(record_size) = geometry {
                    let mut size_updates: Vec<(u64, u64)> = Vec::with_capacity(pending_frns.len());
                    let mut unresolved_frns: Vec<u64> = Vec::new();
                    for &frn in &pending_frns {
                        if let Some(size) = crate::mft_reader::read_single_file_size(
                            volume_handle,
                            frn,
                            record_size,
                        ) {
                            size_updates.push((frn, size));
                        } else {
                            unresolved_frns.push(frn);
                        }
                    }

                    // Apply sizes under lock. If the lock is contended, keep
                    // the FRNs pending so this best-effort refresh is retried
                    // instead of permanently losing a size update.
                    if !size_updates.is_empty() || !unresolved_frns.is_empty() {
                        let mut applied_updates = false;
                        if let Some(mut vol) =
                            handle.try_write_for(INCREMENTAL_WRITE_FALLBACK_TIMEOUT)
                        {
                            let mut changed = false;
                            for (frn, size) in &size_updates {
                                if let Some(rec) = vol.records.get_mut(frn) {
                                    if rec.size != *size {
                                        rec.size = *size;
                                        changed = true;
                                    }
                                }
                            }
                            unresolved_frns.retain(|frn| vol.records.get(frn).is_some());
                            vol.pending_size_refresh.extend(unresolved_frns);
                            if changed {
                                vol.binary_dirty = true;
                            }
                            applied_updates = true;
                        }

                        if !applied_updates {
                            let mut vol = handle.write();
                            vol.pending_size_refresh.extend(pending_frns);
                        }
                    }
                } else {
                    let mut vol = handle.write();
                    vol.pending_size_refresh.extend(pending_frns);
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
            let snapshot = take_pending_snapshot(&handle);
            let should_shrink_index =
                persist_pending_snapshot(db.as_ref(), &handle, drive_letter, snapshot);
            last_persist = std::time::Instant::now();

            // SEC: Prune stale dir_modified_at entries to prevent unbounded memory growth.
            // 10 minutes is generous enough to cover any realistic CheckPathsModified threshold.
            {
                let mut vol = handle.write();
                vol.prune_old_modifications(std::time::Duration::from_secs(600));
                vol.dir_modified_at.shrink_to_fit();
                if should_shrink_index {
                    vol.shrink_to_fit();
                }
            }

            flush_binary_snapshot_if_dirty(&handle, drive_letter);
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

    let final_snapshot = take_pending_snapshot(&handle);
    let _ = persist_pending_snapshot(db.as_ref(), &handle, drive_letter, final_snapshot);
    flush_binary_snapshot_if_dirty(&handle, drive_letter);

    usn_journal::close_volume(volume_handle);
    eprintln!("[USN] {}:\\ Indexer stopped", drive_letter);
}
