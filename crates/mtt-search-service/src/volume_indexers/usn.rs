use std::sync::Arc;

use crate::file_index;
use crate::index_db;
use crate::indexing_progress::IndexingProgress;
use crate::usn_journal;
use crate::volume_indices::SharedVolumeIndices;
use crate::volume_lifecycle::VolumeStopSignal;

mod cache;
mod filesystem_fallback;
mod full_scan;
mod incremental;
mod persistence;
mod ready;
mod size_refresh;
mod volume_io;

pub(crate) fn index_volume(
    volume: super::IndexVolume,
    indices: SharedVolumeIndices,
    indexing_progress: Arc<IndexingProgress>,
    db: Arc<index_db::IndexDb>,
    shutdown: VolumeStopSignal,
) {
    let super::IndexVolume {
        drive_letter,
        volume_id,
        volume_serial,
        ..
    } = volume.clone();
    if shutdown.is_stopped() {
        indexing_progress.clear(drive_letter);
        return;
    }
    let startup_operation = crate::memory_trim::begin_active_operation("USN index startup");
    eprintln!("[USN] Starting indexing for volume {}:\\", drive_letter);
    let filesystem_fallback = filesystem_fallback::FilesystemFallback::new(
        volume,
        indices.clone(),
        indexing_progress.clone(),
        db.clone(),
        shutdown.clone(),
    );

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

    let journal_info =
        match volume_io::with_volume(drive_letter, &shutdown, usn_journal::query_usn_journal) {
            Ok(info) => info,
            Err(e) => {
                indexing_progress.set_error(drive_letter, 0, "query_journal");
                eprintln!(
                    "[USN] Failed to query USN journal for {}:\\: {}",
                    drive_letter, e
                );
                drop(startup_operation);
                filesystem_fallback.run("USN journal unavailable");
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
                if !cache::has_usable_cached_records(&index, state.files_indexed) {
                    eprintln!(
                        "[USN] {}:\\ Cached snapshot is empty; forcing a fresh scan",
                        drive_letter
                    );
                    index.clear();
                    need_full_scan = true;
                } else if !loaded_from_binary && count as u64 != state.files_indexed {
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
                    match volume_io::with_volume(drive_letter, &shutdown, |volume_handle| {
                        usn_journal::read_usn_changes(
                            volume_handle,
                            &journal_info,
                            index.last_usn,
                            &mut index,
                        )
                    }) {
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
        index = match full_scan::build_and_persist(
            drive_letter,
            &volume_id,
            volume_serial,
            &journal_info,
            &indexing_progress,
            &db,
            &shutdown,
        ) {
            Ok(index) => index,
            Err(_error) => {
                if shutdown.is_stopped() {
                    indexing_progress.clear(drive_letter);
                    return;
                }
                drop(startup_operation);
                filesystem_fallback.run("initial MFT scan failed");
                return;
            }
        };
    }

    if shutdown.is_stopped() {
        indexing_progress.clear(drive_letter);
        return;
    }

    drop(startup_operation);
    ready::publish_and_run(
        drive_letter,
        index,
        journal_info,
        indices,
        indexing_progress,
        db,
        shutdown,
    );
}
