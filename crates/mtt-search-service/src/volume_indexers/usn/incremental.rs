use std::sync::Arc;

use crate::index_db;
use crate::usn_journal::UsnJournalInfo;
use crate::volume_indices::VolumeIndexHandle;
use crate::volume_lifecycle::VolumeStopSignal;

use super::persistence::{
    flush_binary_snapshot_if_dirty, persist_pending_snapshot, take_pending_snapshot,
    SIZE_REFRESH_BATCH_RECORDS,
};

const INCREMENTAL_APPLY_RETRY_ATTEMPTS: usize = 3;
const INCREMENTAL_APPLY_RETRY_SLEEP: std::time::Duration = std::time::Duration::from_millis(35);
const INCREMENTAL_WRITE_FALLBACK_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(60);
const INCREMENTAL_CONTENTION_LOG_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

pub(super) fn run_incremental(
    drive_letter: char,
    mut current_usn: i64,
    journal_info: UsnJournalInfo,
    handle: VolumeIndexHandle,
    db: Arc<index_db::IndexDb>,
    shutdown: VolumeStopSignal,
) {
    let mut last_persist = std::time::Instant::now();
    let mut contention_retries = 0u64;
    let mut contention_applied_after_retry = 0u64;
    let mut contention_skipped_cycles = 0u64;
    let mut last_contention_log = std::time::Instant::now();

    'incremental: loop {
        if shutdown.wait_or_stop(std::time::Duration::from_secs(2)) {
            break;
        }

        match super::volume_io::with_volume(drive_letter, &shutdown, |volume_handle| {
            crate::usn_journal::read_usn_buffer(volume_handle, &journal_info, current_usn)
        }) {
            Ok(Some((buffer, bytes_returned, new_usn))) => {
                let _trim_exclusion = crate::memory_trim::begin_trim_exclusion();
                let mut applied = false;
                let mut parse_failed = false;
                let mut attempt = 0usize;
                while attempt < INCREMENTAL_APPLY_RETRY_ATTEMPTS {
                    match handle.try_write() {
                        Some(mut vol_index) => {
                            let mut dummy_count = 0;
                            let parse_result = crate::usn_journal::parse_usn_records(
                                &buffer[8..bytes_returned as usize],
                                &mut vol_index,
                                &mut dummy_count,
                                true,
                            );
                            match parse_result {
                                Ok(()) => {
                                    vol_index.last_usn = new_usn;
                                    current_usn = new_usn;
                                    applied = true;
                                    if attempt > 0 {
                                        contention_applied_after_retry += 1;
                                    }
                                }
                                Err(e) => {
                                    parse_failed = true;
                                    eprintln!(
                                        "[USN] {}:\\ Incremental apply failed: {}",
                                        drive_letter, e
                                    );
                                }
                            }
                            break;
                        }
                        None => {
                            attempt += 1;
                            if attempt < INCREMENTAL_APPLY_RETRY_ATTEMPTS {
                                contention_retries += 1;
                                if shutdown.wait_or_stop(INCREMENTAL_APPLY_RETRY_SLEEP) {
                                    break;
                                }
                            }
                        }
                    }
                }

                if !applied && !parse_failed {
                    match handle.try_write_for(INCREMENTAL_WRITE_FALLBACK_TIMEOUT) {
                        Some(mut vol_index) => {
                            let mut dummy_count = 0;
                            let parse_result = crate::usn_journal::parse_usn_records(
                                &buffer[8..bytes_returned as usize],
                                &mut vol_index,
                                &mut dummy_count,
                                true,
                            );
                            match parse_result {
                                Ok(()) => {
                                    vol_index.last_usn = new_usn;
                                    current_usn = new_usn;
                                }
                                Err(e) => eprintln!(
                                    "[USN] {}:\\ Incremental apply failed: {}",
                                    drive_letter, e
                                ),
                            }
                        }
                        None => contention_skipped_cycles += 1,
                    }
                }
            }
            Ok(None) => {}
            Err(e) => eprintln!(
                "[USN] {}:\\ Incremental read error: {}",
                drive_letter,
                crate::redact_paths(&e)
            ),
        }

        if shutdown.is_stopped() {
            break 'incremental;
        }

        {
            let pending_frns: Vec<u64> = match handle.try_write() {
                Some(mut vol) => {
                    if vol.sizes_loaded && !vol.pending_size_refresh.is_empty() {
                        let pending: Vec<u64> = vol
                            .pending_size_refresh
                            .iter()
                            .take(SIZE_REFRESH_BATCH_RECORDS)
                            .copied()
                            .collect();
                        for frn in &pending {
                            vol.pending_size_refresh.remove(frn);
                        }
                        vol.size_refresh_in_progress = true;
                        pending
                    } else {
                        Vec::new()
                    }
                }
                None => Vec::new(),
            };

            if !pending_frns.is_empty() {
                let _trim_exclusion = crate::memory_trim::begin_trim_exclusion();
                let metrics =
                    super::volume_io::with_volume(drive_letter, &shutdown, |volume_handle| {
                        let record_size = crate::mft_reader::query_mft_geometry_pub(volume_handle)?;
                        let mut size_updates = Vec::with_capacity(pending_frns.len());
                        let mut unresolved_frns = Vec::new();
                        for &frn in &pending_frns {
                            if shutdown.is_stopped() {
                                return Err("Size refresh cancelled".to_string());
                            }
                            if let Some(metrics) = crate::mft_reader::read_single_file_metrics(
                                volume_handle,
                                frn,
                                record_size,
                            ) {
                                size_updates.push((frn, metrics));
                            } else {
                                unresolved_frns.push(frn);
                            }
                        }
                        Ok((size_updates, unresolved_frns))
                    });
                if let Ok((size_updates, mut unresolved_frns)) = metrics {
                    if shutdown.is_stopped() {
                        let mut vol = handle.write();
                        vol.pending_size_refresh
                            .extend(pending_frns.iter().copied());
                        vol.size_refresh_in_progress = false;
                        break 'incremental;
                    }

                    if !size_updates.is_empty() || !unresolved_frns.is_empty() {
                        let mut applied_updates = false;
                        if let Some(mut vol) =
                            handle.try_write_for(INCREMENTAL_WRITE_FALLBACK_TIMEOUT)
                        {
                            let mut changed = false;
                            for (frn, metrics) in &size_updates {
                                if let Some(rec) = vol.records.get_mut(frn) {
                                    if (rec.size, rec.allocated_size)
                                        != (metrics.logical_size, metrics.allocated_size)
                                    {
                                        rec.size = metrics.logical_size;
                                        rec.allocated_size = metrics.allocated_size;
                                        changed = true;
                                    }
                                }
                            }
                            unresolved_frns.retain(|frn| vol.records.get(frn).is_some());
                            vol.pending_size_refresh.extend(unresolved_frns);
                            vol.size_refresh_in_progress = false;
                            if changed {
                                vol.binary_dirty = true;
                            }
                            applied_updates = true;
                        }

                        if !applied_updates {
                            let mut vol = handle.write();
                            vol.pending_size_refresh.extend(pending_frns);
                            vol.size_refresh_in_progress = false;
                        }
                    }
                } else {
                    let mut vol = handle.write();
                    vol.pending_size_refresh.extend(pending_frns);
                    vol.size_refresh_in_progress = false;
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

        if last_persist.elapsed() > std::time::Duration::from_secs(300) {
            let _trim_exclusion = crate::memory_trim::begin_trim_exclusion();
            match take_pending_snapshot(&handle) {
                Ok(snapshot) => {
                    persist_pending_snapshot(db.as_ref(), &handle, drive_letter, snapshot);
                }
                Err(error) => eprintln!(
                    "[USN] {}:\\ Refusing invalid persistence snapshot: {}",
                    drive_letter,
                    crate::redact_paths(&error)
                ),
            }
            last_persist = std::time::Instant::now();

            {
                let mut vol = handle.write();
                vol.prune_old_modifications(std::time::Duration::from_secs(600));
                vol.dir_modified_at.shrink_to_fit();
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

    if shutdown.was_requested() {
        eprintln!(
            "[USN] {}:\\ Releasing volume at the last durable checkpoint; pending journal changes will be replayed on return",
            drive_letter
        );
        return;
    }

    match take_pending_snapshot(&handle) {
        Ok(final_snapshot) => {
            let _ = persist_pending_snapshot(db.as_ref(), &handle, drive_letter, final_snapshot);
        }
        Err(error) => eprintln!(
            "[USN] {}:\\ Refusing invalid final persistence snapshot: {}",
            drive_letter,
            crate::redact_paths(&error)
        ),
    }
    flush_binary_snapshot_if_dirty(&handle, drive_letter);

    eprintln!("[USN] {}:\\ Indexer stopped", drive_letter);
}
