use std::collections::HashSet;

use crate::index_db;
use crate::volume_indices::VolumeIndexHandle;

pub(super) const SIZE_REFRESH_BATCH_RECORDS: usize = 4_096;

pub(super) struct PendingPersistSnapshot {
    pub(super) drive_letter: char,
    pub(super) volume_id: String,
    pub(super) volume_serial: u32,
    pub(super) journal_id: u64,
    pub(super) last_usn: i64,
    pub(super) files_indexed: usize,
    pub(super) additions: HashSet<u64>,
    pub(super) removals: HashSet<u64>,
    pub(super) addition_rows: Vec<(u64, String, u64, bool, bool, Vec<u64>)>,
}

fn restore_pending_snapshot(handle: &VolumeIndexHandle, snapshot: PendingPersistSnapshot) {
    let mut vol_index = handle.write();
    vol_index.pending_additions.extend(snapshot.additions);
    vol_index.pending_removals.extend(snapshot.removals);
}

pub(super) fn take_pending_snapshot(
    handle: &VolumeIndexHandle,
) -> Result<PendingPersistSnapshot, String> {
    let mut vol_index = handle.write();
    let addition_rows: Vec<_> = vol_index
        .pending_additions
        .iter()
        .map(|frn| {
            let record = vol_index
                .records
                .get(frn)
                .ok_or_else(|| format!("Pending FRN {frn} is missing from the index"))?;
            let name = vol_index
                .names
                .try_get(record.name_ref())
                .ok_or_else(|| format!("Pending FRN {frn} has an invalid NameRef"))?;
            Ok((
                *frn,
                name.to_string(),
                record.parent_ref,
                record.is_dir(),
                vol_index.reparse_points.contains(frn),
                vol_index
                    .hardlink_parents
                    .get(frn)
                    .cloned()
                    .unwrap_or_default(),
            ))
        })
        .collect::<Result<_, String>>()?;
    let additions = std::mem::take(&mut vol_index.pending_additions);
    let removals = std::mem::take(&mut vol_index.pending_removals);

    Ok(PendingPersistSnapshot {
        drive_letter: vol_index.drive_letter,
        volume_id: vol_index.volume_id.clone(),
        volume_serial: vol_index.volume_serial,
        journal_id: vol_index.journal_id,
        last_usn: vol_index.last_usn,
        files_indexed: vol_index.records.len(),
        additions,
        removals,
        addition_rows,
    })
}

pub(super) fn persist_pending_snapshot(
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

    if skip_sqlite_data {
        if let Err(e) = db.save_volume_state_snapshot(
            snapshot.drive_letter,
            &snapshot.volume_id,
            snapshot.volume_serial,
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
            restore_pending_snapshot(handle, snapshot);
            return false;
        }
    } else if let Err(e) = db.sync_volume_incremental_snapshot(
        &index_db::PersistedVolumeState {
            drive_letter: snapshot.drive_letter,
            volume_id: snapshot.volume_id.clone(),
            volume_serial: snapshot.volume_serial,
            journal_id: snapshot.journal_id,
            last_usn: snapshot.last_usn,
            files_indexed: snapshot.files_indexed as u64,
            has_hardlink_parent_data: true,
            has_reparse_point_data: true,
        },
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

    had_structural_changes
}

pub(super) fn flush_binary_snapshot_if_dirty(
    handle: &VolumeIndexHandle,
    drive_letter: char,
) -> bool {
    {
        let vol = handle.read();
        if !vol.binary_dirty {
            return true;
        }
        if !vol.sizes_loaded || vol.has_pending_size_refreshes() {
            return false;
        }
    }

    let _snapshot_permit = super::super::begin_binary_snapshot();
    let should_trim = {
        let mut vol = handle.write();
        if !vol.binary_dirty {
            return true;
        }
        if !vol.sizes_loaded {
            return false;
        }
        if vol.has_pending_size_refreshes() {
            return false;
        }

        match crate::index_db::binary::save_and_remap(&mut vol) {
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
        crate::memory_trim::request_trim(format!("{}:\\ binary snapshot flush", drive_letter));
    }
    should_trim
}

pub(super) fn should_prefer_sqlite_over_binary(
    sqlite_state: &index_db::PersistedVolumeState,
    binary_state: &index_db::PersistedVolumeState,
    current_journal_id: u64,
) -> bool {
    let sqlite_matches_current = sqlite_state.journal_id == current_journal_id;
    let binary_matches_current = binary_state.journal_id == current_journal_id;
    let sqlite_is_fresher_same_journal = !crate::index_db::skip_sqlite_data_persistence()
        && sqlite_state.journal_id == binary_state.journal_id
        && sqlite_state.last_usn > binary_state.last_usn;

    sqlite_is_fresher_same_journal || (sqlite_matches_current && !binary_matches_current)
}
