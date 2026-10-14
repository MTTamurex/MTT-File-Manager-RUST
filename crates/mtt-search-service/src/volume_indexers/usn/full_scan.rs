use std::sync::Arc;

use windows::Win32::Foundation::HANDLE;

use crate::file_index::VolumeIndex;
use crate::index_db;
use crate::indexing_progress::IndexingProgress;
use crate::usn_journal::UsnJournalInfo;
use crate::volume_lifecycle::VolumeStopSignal;

pub(super) fn build_and_persist(
    drive_letter: char,
    volume_id: &str,
    volume_serial: u32,
    volume_handle: HANDLE,
    journal_info: &UsnJournalInfo,
    indexing_progress: &Arc<IndexingProgress>,
    db: &Arc<index_db::IndexDb>,
    stop: &VolumeStopSignal,
) -> Result<VolumeIndex, String> {
    crate::memory_trim::log_process_memory(&format!("{}:\\ before full MFT scan", drive_letter));
    indexing_progress.set_scanning(drive_letter, 0, "scanning_mft");
    eprintln!("[USN] {}:\\ Starting bulk MFT read...", drive_letter);
    let start = std::time::Instant::now();

    let mut index = match crate::mft_reader::read_mft_bulk(
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
    ) {
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
