use crate::file_index::VolumeIndex;
use crate::index_db::{self, PersistedVolumeState};
use crate::volume_lifecycle::VolumeStopSignal;

pub(super) struct LoadedVolumeCache {
    pub(super) index: VolumeIndex,
    pub(super) sqlite_state: Option<PersistedVolumeState>,
    pub(super) binary_candidate: Option<(VolumeIndex, PersistedVolumeState)>,
}

pub(super) fn load_volume_cache(
    drive_letter: char,
    volume_id: &str,
    volume_serial: u32,
    db: &index_db::IndexDb,
    stop: &VolumeStopSignal,
) -> Option<LoadedVolumeCache> {
    let mut index = VolumeIndex::empty(drive_letter);
    index.volume_id = volume_id.to_string();
    index.volume_serial = volume_serial;
    let sqlite_state = db.load_volume_state(drive_letter).filter(|state| {
        cache_identity_matches(
            &state.volume_id,
            state.volume_serial,
            volume_id,
            volume_serial,
        )
    });

    let binary_candidate = if sqlite_state.is_some() {
        let _build_permit = super::super::wait_for_index_build(stop)?;
        if stop.is_stopped() {
            return None;
        }
        match crate::index_db::binary::load(drive_letter) {
            Ok(Some((binary_index, binary_state)))
                if cache_identity_matches(
                    &binary_state.volume_id,
                    binary_state.volume_serial,
                    volume_id,
                    volume_serial,
                ) =>
            {
                Some((
                    binary_index,
                    PersistedVolumeState {
                        drive_letter,
                        volume_id: binary_state.volume_id,
                        volume_serial: binary_state.volume_serial,
                        journal_id: binary_state.journal_id,
                        last_usn: binary_state.last_usn,
                        files_indexed: binary_state.files_indexed,
                        has_hardlink_parent_data: binary_state.has_hardlink_parent_data,
                        has_reparse_point_data: binary_state.has_reparse_point_data,
                    },
                ))
            }
            Ok(Some((_binary_index, _binary_state))) => {
                eprintln!(
                    "[USN] {}:\\ Binary snapshot volume identity mismatch; rebuilding from the current volume",
                    drive_letter
                );
                None
            }
            Ok(None) => None,
            Err(error) => {
                eprintln!(
                    "[USN] {}:\\ Binary index load failed ({}), trying SQLite",
                    drive_letter, error
                );
                None
            }
        }
    } else {
        None
    };

    Some(LoadedVolumeCache {
        index,
        sqlite_state,
        binary_candidate,
    })
}

pub(super) fn has_usable_cached_records(index: &VolumeIndex, persisted_records: u64) -> bool {
    persisted_records > 0 && !index.records.is_empty()
}

fn cache_identity_matches(
    stored_id: &str,
    stored_serial: u32,
    current_id: &str,
    current_serial: u32,
) -> bool {
    current_serial != 0
        && stored_serial == current_serial
        && stored_id.eq_ignore_ascii_case(current_id)
}

#[cfg(test)]
mod tests {
    use super::{cache_identity_matches, has_usable_cached_records};
    use crate::file_index::VolumeIndex;

    #[test]
    fn cache_requires_a_known_matching_volume_serial() {
        assert!(cache_identity_matches(
            "volume-id",
            0x1234,
            "VOLUME-ID",
            0x1234
        ));
        assert!(!cache_identity_matches("volume-id", 0, "volume-id", 0x1234));
        assert!(!cache_identity_matches("volume-id", 0x1234, "volume-id", 0));
        assert!(!cache_identity_matches(
            "volume-id",
            0x1234,
            "volume-id",
            0x5678
        ));
    }

    #[test]
    fn empty_persisted_or_loaded_snapshot_requires_a_rescan() {
        let empty = VolumeIndex::empty('R');
        assert!(!has_usable_cached_records(&empty, 0));
        assert!(!has_usable_cached_records(&empty, 25));

        let mut index = VolumeIndex::empty('R');
        assert!(index.insert_record_untracked(6, "file.txt", 5, false, false));
        assert!(!has_usable_cached_records(&index, 0));
        assert!(has_usable_cached_records(&index, 1));
    }
}
