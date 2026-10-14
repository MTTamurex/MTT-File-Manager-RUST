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
    db: &index_db::IndexDb,
    stop: &VolumeStopSignal,
) -> Option<LoadedVolumeCache> {
    let mut index = VolumeIndex::empty(drive_letter);
    index.volume_id = volume_id.to_string();
    let sqlite_state = db
        .load_volume_state(drive_letter)
        .filter(|state| state.volume_id.eq_ignore_ascii_case(volume_id));

    let binary_candidate = if sqlite_state.is_some() {
        let _build_permit = super::super::wait_for_index_build(stop)?;
        if stop.is_stopped() {
            return None;
        }
        match crate::index_db::binary::load(drive_letter) {
            Ok(Some((binary_index, binary_state)))
                if binary_state.volume_id.eq_ignore_ascii_case(volume_id) =>
            {
                Some((
                    binary_index,
                    PersistedVolumeState {
                        drive_letter,
                        volume_id: binary_state.volume_id,
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
