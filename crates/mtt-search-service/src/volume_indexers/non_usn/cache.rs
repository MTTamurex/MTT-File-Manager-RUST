use crate::file_index::{IndexState, VolumeIndex};
use crate::index_db::IndexDb;
use crate::volume_indexers::IndexVolume;

pub(super) fn matching_record_estimate(db: &IndexDb, volume: &IndexVolume) -> Option<usize> {
    let state = db.load_volume_state(volume.drive_letter);
    crate::startup_trace::event(
        Some(volume.drive_letter),
        "fallback_cache_identity",
        &[
            ("metadata_exists", u64::from(state.is_some())),
            ("serial_known", u64::from(volume.volume_serial != 0)),
            ("serial_matches", u64::from(state.as_ref().is_some_and(|s| s.volume_serial == volume.volume_serial))),
            ("identity_matches", u64::from(state.as_ref().is_some_and(|s| s.volume_id.eq_ignore_ascii_case(&volume.volume_id)))),
        ],
    );
    state
        .filter(|state| {
            volume.volume_serial != 0
                && state.volume_id.eq_ignore_ascii_case(&volume.volume_id)
                && state.volume_serial == volume.volume_serial
        })
        .map(|state| state.files_indexed.min(usize::MAX as u64) as usize)
}

pub(super) fn load_cached_index(
    db: &IndexDb,
    volume: &IndexVolume,
    estimate: Option<usize>,
) -> Option<VolumeIndex> {
    // Rows without matching volume metadata may belong to another medium.
    let mut index = VolumeIndex::with_estimated_records(volume.drive_letter, estimate?);
    index.volume_id = volume.volume_id.clone();
    index.volume_serial = volume.volume_serial;
    db.load_into_index(&mut index, |_| {}).ok().flatten()?;
    index.shrink_to_fit();
    index.journal_id = 0;
    index.last_usn = 0;
    index.state = IndexState::Ready;
    Some(index)
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
