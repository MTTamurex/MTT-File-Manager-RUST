use std::sync::Arc;

use crate::file_index::{self, VolumeIndex};
use crate::index_db;
use crate::indexing_progress::IndexingProgress;
use crate::usn_journal::UsnJournalInfo;
use crate::volume_indices::{self, SharedVolumeIndices};
use crate::volume_lifecycle::VolumeStopSignal;

pub(super) fn publish_and_run(
    drive_letter: char,
    mut index: VolumeIndex,
    journal_info: UsnJournalInfo,
    indices: SharedVolumeIndices,
    indexing_progress: Arc<IndexingProgress>,
    db: Arc<index_db::IndexDb>,
    shutdown: VolumeStopSignal,
) {
    index.shrink_to_fit();
    index.state = file_index::IndexState::Ready;
    let current_usn = index.last_usn;
    let sizes_already_loaded = index.sizes_loaded;

    let handle = volume_indices::upsert(&indices, index);
    indexing_progress.clear(drive_letter);
    if sizes_already_loaded {
        crate::memory_trim::request_trim(format!("{}:\\ index ready", drive_letter));
    }

    let size_worker = (!sizes_already_loaded).then(|| {
        super::size_refresh::spawn_background_size_refresh(
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
    super::incremental::run_incremental(
        drive_letter,
        current_usn,
        journal_info,
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
