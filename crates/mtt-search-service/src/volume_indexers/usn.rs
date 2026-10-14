use std::sync::Arc;

use crate::file_index;
use crate::index_db;
use crate::indexing_progress::IndexingProgress;
use crate::usn_journal;
use crate::volume_indices::{self, SharedVolumeIndices, VolumeIndexHandle};
use crate::volume_lifecycle::VolumeStopSignal;

mod cache;
mod full_scan;
mod incremental;
mod persistence;
mod size_refresh;

pub(crate) fn index_volume(
    drive_letter: char,
    volume_id: String,
    volume_serial: u32,
    indices: SharedVolumeIndices,
    indexing_progress: Arc<IndexingProgress>,
    db: Arc<index_db::IndexDb>,
    shutdown: VolumeStopSignal,
) {
    if shutdown.is_stopped() {
        indexing_progress.clear(drive_letter);
        return;
    }
    let startup_operation = crate::memory_trim::begin_active_operation("USN index startup");
    eprintln!("[USN] Starting indexing for volume {}:\\", drive_letter);

    let Some(cache::LoadedVolumeCache {
        mut index,
        sqlite_state,
        binary_candidate,
    }) = cache::load_volume_cache(drive_letter, &volume_id, volume_serial, &db, &shutdown)
    else {
        indexing_progress.clear(drive_letter);
        return;
    };
    index.volume_serial = volume_serial;
    if shutdown.is_stopped() {
        indexing_progress.clear(drive_letter);
        return;
    }

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
            if persistence::should_prefer_sqlite_over_binary(
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
            index.volume_id = volume_id.clone();
            index.volume_serial = volume_serial;
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
                index.volume_id = volume_id.clone();
                index.volume_serial = volume_serial;
                indexing_progress.update(
                    drive_letter,
                    "scanning",
                    0,
                    "loading_cache",
                    Some(0),
                    Some(state.files_indexed),
                );
                let Some(_build_permit) = super::wait_for_index_build(&shutdown) else {
                    usn_journal::close_volume(volume_handle);
                    indexing_progress.clear(drive_letter);
                    return;
                };
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
        let Some(_build_permit) = super::wait_for_index_build(&shutdown) else {
            usn_journal::close_volume(volume_handle);
            indexing_progress.clear(drive_letter);
            return;
        };
        index = match full_scan::build_and_persist(
            drive_letter,
            &volume_id,
            volume_serial,
            volume_handle,
            &journal_info,
            &indexing_progress,
            &db,
            &shutdown,
        ) {
            Ok(index) => index,
            Err(_error) => {
                if shutdown.is_stopped() {
                    indexing_progress.clear(drive_letter);
                    usn_journal::close_volume(volume_handle);
                    return;
                }
                usn_journal::close_volume(volume_handle);
                return;
            }
        };
    }

    if shutdown.is_stopped() {
        indexing_progress.clear(drive_letter);
        usn_journal::close_volume(volume_handle);
        return;
    }

    index.shrink_to_fit();
    index.state = file_index::IndexState::Ready;
    let current_usn = index.last_usn;
    let sizes_already_loaded = index.sizes_loaded;

    // Register / update this volume's handle. From here on, all per-volume
    // mutations happen through `handle` directly — the outer registry lock is
    // not taken again, so concurrent writers/readers on *other* volumes are
    // never blocked by this thread.
    let handle: VolumeIndexHandle = volume_indices::upsert(&indices, index);
    indexing_progress.clear(drive_letter);
    if sizes_already_loaded {
        crate::memory_trim::request_trim(format!("{}:\\ index ready", drive_letter));
    }

    let size_worker = (!sizes_already_loaded).then(|| {
        size_refresh::spawn_background_size_refresh(
            drive_letter,
            handle.clone(),
            indexing_progress.clone(),
            shutdown.clone(),
        )
    });

    eprintln!(
        "[USN] {}:\\ Index ready, starting incremental updates",
        drive_letter
    );
    drop(startup_operation);
    incremental::run_incremental(
        drive_letter,
        current_usn,
        journal_info,
        volume_handle,
        handle,
        db,
        shutdown,
    );
    if let Some(worker) = size_worker {
        if worker.join().is_err() {
            eprintln!(
                "[MFT-SIZE] {}:\\ Background size worker panicked",
                drive_letter
            );
        }
    }
}
