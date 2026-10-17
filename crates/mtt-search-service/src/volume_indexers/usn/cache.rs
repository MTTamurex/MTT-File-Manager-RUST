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
    load_volume_cache_with(
        drive_letter,
        volume_id,
        volume_serial,
        db,
        stop,
        crate::index_db::binary::load,
    )
}

fn load_volume_cache_with(
    drive_letter: char,
    volume_id: &str,
    volume_serial: u32,
    db: &index_db::IndexDb,
    stop: &VolumeStopSignal,
    load_binary: impl FnOnce(
        char,
    ) -> Result<
        Option<(VolumeIndex, index_db::binary::PersistedBinaryState)>,
        String,
    >,
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

    let binary_candidate = {
        let _build_permit = super::super::wait_for_index_build(stop)?;
        if stop.is_stopped() {
            return None;
        }
        match load_binary(drive_letter) {
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
    fn binary_recovery_does_not_require_sqlite_metadata_and_still_checks_identity() {
        use crate::index_db::{binary::PersistedBinaryState, IndexDb};
        use crate::volume_lifecycle::VolumeLifecycle;
        use std::sync::{atomic::AtomicBool, Arc};

        let db = IndexDb::open(std::path::Path::new(":memory:")).unwrap();
        let (wake, _events) = std::sync::mpsc::channel();
        let lifecycle = VolumeLifecycle::new(
            Arc::new(AtomicBool::new(false)),
            crate::volume_indices::new_shared(),
            Arc::new(crate::indexing_progress::IndexingProgress::new()),
            wake,
        );
        let ticket = lifecycle
            .claim_worker(&crate::usn_journal::DiscoveredVolume {
                drive_letter: 'R',
                volume_id: "volume-id".into(),
                volume_serial: 123,
                label: String::new(),
                file_system: "NTFS".into(),
                usn_supported: true,
            })
            .unwrap();
        let stop = ticket.stop_signal();
        for (stored_id, stored_serial, expected) in [
            ("volume-id", 123, true),
            ("different-volume", 123, false),
            ("volume-id", 456, false),
            ("volume-id", 0, false),
        ] {
            let loaded = super::load_volume_cache_with('R', "volume-id", 123, &db, &stop, |_| {
                let mut index = VolumeIndex::empty('R');
                index.volume_id = stored_id.into();
                index.volume_serial = stored_serial;
                assert!(index.insert_record_untracked(10, "file.txt", 5, false, false));
                Ok(Some((
                    index,
                    PersistedBinaryState {
                        volume_id: stored_id.into(),
                        volume_serial: stored_serial,
                        journal_id: 7,
                        last_usn: 8,
                        files_indexed: 1,
                        has_hardlink_parent_data: true,
                        has_reparse_point_data: true,
                    },
                )))
            })
            .unwrap();
            assert!(loaded.sqlite_state.is_none());
            assert_eq!(loaded.binary_candidate.is_some(), expected);
            if let Some((index, state)) = loaded.binary_candidate {
                assert_eq!(index.records.len(), 1);
                assert_eq!((state.journal_id, state.last_usn), (7, 8));
            }
        }
    }

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
