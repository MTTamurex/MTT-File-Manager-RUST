use std::sync::Arc;

use crate::file_index::VolumeIndex;
use crate::index_db;
use crate::indexing_progress::IndexingProgress;
use crate::usn_journal::UsnJournalInfo;
use crate::volume_lifecycle::VolumeStopSignal;

pub(super) fn build_and_persist(
    drive_letter: char,
    volume_id: &str,
    volume_serial: u32,
    journal_info: &UsnJournalInfo,
    indexing_progress: &Arc<IndexingProgress>,
    db: &Arc<index_db::IndexDb>,
    stop: &VolumeStopSignal,
) -> Result<VolumeIndex, String> {
    // The permit belongs to the MFT attempt, not to the caller that may start
    // a filesystem fallback after this function returns an error.
    let _build_permit = super::super::wait_for_index_build(stop)
        .ok_or_else(|| "MFT scan cancelled by volume removal".to_string())?;
    crate::memory_trim::log_process_memory(&format!("{}:\\ before full MFT scan", drive_letter));
    indexing_progress.set_scanning(drive_letter, 0, "scanning_mft");
    eprintln!("[USN] {}:\\ Starting bulk MFT read...", drive_letter);
    let start = std::time::Instant::now();

    let index = match super::volume_io::with_volume(drive_letter, stop, |volume_handle| {
        crate::mft_reader::read_mft_bulk(
            volume_handle,
            drive_letter,
            |done, total, indexed| {
                indexing_progress.update(
                    drive_letter,
                    "scanning",
                    indexed,
                    "scanning_mft",
                    Some(done),
                    Some(total),
                )
            },
            || stop.is_stopped(),
        )
    }) {
        Ok(index) => index,
        Err(error) => {
            if !stop.is_stopped() {
                indexing_progress.set_error(drive_letter, 0, "scanning_mft");
                eprintln!(
                    "[USN] {}:\\ Bulk MFT read failed: {}",
                    drive_letter,
                    crate::redact_paths(&error)
                );
            }
            return Err(error);
        }
    };

    let mut index = match validate_mft_index(index) {
        Ok(index) => index,
        Err(error) => {
            indexing_progress.set_error(drive_letter, 0, "scanning_mft");
            eprintln!("[USN] {}:\\ {}", drive_letter, error);
            return Err(error);
        }
    };

    if stop.is_stopped() {
        return Err("MFT scan cancelled by volume removal".to_string());
    }

    index.volume_id = volume_id.to_string();
    index.volume_serial = volume_serial;
    let elapsed = start.elapsed();
    index.journal_id = journal_info.journal_id;
    index.last_usn = journal_info.next_usn;
    index.hardlink_data_complete = true;
    index.reparse_data_complete = true;
    index.sizes_loaded = true;
    eprintln!(
        "[USN] {}:\\ Bulk MFT read: {} files in {:.2}s",
        drive_letter,
        index.records.len(),
        elapsed.as_secs_f64()
    );

    let arena_before = index.names.len();
    let referenced_name_bytes = index.referenced_name_bytes();
    let dead_name_bytes = arena_before.saturating_sub(referenced_name_bytes);
    let (_arena_used, arena_cap, records_est) = index.memory_usage();
    eprintln!(
        "[USN] {}:\\ Arena before streaming checkpoint: {:.1} MB, live {:.1} MB, dead {:.1} MB, records ~{:.1} MB, total ~{:.1} MB",
        drive_letter,
        arena_before as f64 / 1_048_576.0,
        referenced_name_bytes as f64 / 1_048_576.0,
        dead_name_bytes as f64 / 1_048_576.0,
        records_est as f64 / 1_048_576.0,
        (arena_cap + records_est) as f64 / 1_048_576.0
    );
    if !index.hardlink_parents.is_empty() {
        eprintln!(
            "[USN] {}:\\ {} hardlinked files with {} extra parent entries",
            drive_letter,
            index.hardlink_parents.len(),
            index
                .hardlink_parents
                .values()
                .map(|parents| parents.len())
                .sum::<usize>()
        );
    }

    indexing_progress.update(
        drive_letter,
        "scanning",
        index.records.len() as u64,
        "persisting",
        None,
        None,
    );
    let _snapshot_permit = super::super::begin_binary_snapshot();
    let binary_saved = match crate::index_db::binary::save_and_remap(&mut index) {
        Ok(()) => {
            index.binary_dirty = false;
            true
        }
        Err(error) => {
            eprintln!(
                "[USN] {}:\\ Binary save failed: {}",
                drive_letter,
                crate::redact_paths(&error)
            );
            false
        }
    };
    let skip_sqlite_data = crate::index_db::skip_sqlite_data_persistence();

    if !skip_sqlite_data || binary_saved {
        if let Err(error) = db.save_volume_state_snapshot(
            drive_letter,
            volume_id,
            volume_serial,
            index.journal_id,
            index.last_usn,
            index.records.len(),
            index.hardlink_data_complete,
            index.reparse_data_complete,
        ) {
            eprintln!(
                "[USN] {}:\\ SQLite volume state snapshot failed: {}",
                drive_letter,
                crate::redact_paths(&error)
            );
        }
    }

    if skip_sqlite_data {
        if binary_saved {
            db.purge_volume_data(drive_letter);
        } else {
            eprintln!(
                "[USN] {}:\\ Keeping existing SQLite data because binary save failed",
                drive_letter
            );
        }
    } else {
        indexing_progress.update(
            drive_letter,
            "scanning",
            index.records.len() as u64,
            "persisting_sqlite",
            None,
            None,
        );
        if let Err(error) = db.save_volume(&index, |inserted, total| {
            indexing_progress.update(
                drive_letter,
                "scanning",
                index.records.len() as u64,
                "persisting_sqlite",
                Some(inserted),
                Some(total),
            )
        }) {
            eprintln!(
                "[USN] {}:\\ SQLite full snapshot save failed: {}",
                drive_letter,
                crate::redact_paths(&error)
            );
        }
    }

    index.clear_pending();
    crate::memory_trim::log_process_memory(&format!("{}:\\ after full MFT scan", drive_letter));
    Ok(index)
}

fn validate_mft_index(index: VolumeIndex) -> Result<VolumeIndex, String> {
    if index.records.is_empty() {
        Err("MFT scan returned no indexable records".to_string())
    } else {
        Ok(index)
    }
}

#[cfg(test)]
mod tests {
    use super::{validate_mft_index, VolumeIndex};
    use crate::index_db::IndexDb;
    use crate::indexing_progress::IndexingProgress;
    use crate::usn_journal::{DiscoveredVolume, UsnJournalInfo};
    use crate::volume_lifecycle::VolumeLifecycle;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;
    use std::sync::{mpsc, Arc};

    #[test]
    fn empty_mft_result_is_rejected_instead_of_persisted_as_a_ready_index() {
        match validate_mft_index(VolumeIndex::empty('R')) {
            Err(error) => assert_eq!(error, "MFT scan returned no indexable records"),
            Ok(_) => panic!("empty MFT results must trigger the filesystem fallback"),
        }
    }

    #[test]
    fn failed_mft_attempt_releases_build_permit_before_fallback() {
        let progress = Arc::new(IndexingProgress::new());
        let (wake, _events) = mpsc::channel();
        let lifecycle = VolumeLifecycle::new(
            Arc::new(AtomicBool::new(false)),
            crate::volume_indices::new_shared(),
            progress.clone(),
            wake,
        );
        // An invalid drive letter guarantees the attempt fails without touching
        // any mounted volume or requiring physical test media.
        let volume = DiscoveredVolume {
            drive_letter: '?',
            volume_id: "test-volume".to_string(),
            volume_serial: 1,
            label: String::new(),
            file_system: "NTFS".to_string(),
            usn_supported: true,
        };
        let ticket = lifecycle.claim_worker(&volume).unwrap();
        let db = Arc::new(IndexDb::open(Path::new(":memory:")).unwrap());
        let journal = UsnJournalInfo {
            journal_id: 1,
            first_usn: 0,
            next_usn: 0,
        };

        assert!(super::build_and_persist(
            volume.drive_letter,
            &volume.volume_id,
            volume.volume_serial,
            &journal,
            &progress,
            &db,
            &ticket.stop_signal(),
        )
        .is_err());
        let fallback_permit = crate::volume_indexers::try_begin_index_build();
        assert!(
            fallback_permit.is_some(),
            "MFT failure must not retain the build lock"
        );
        drop(fallback_permit);
        drop(ticket);
        assert!(lifecycle.prepare_remove_identity(&volume.volume_id, volume.volume_serial));
    }
}
