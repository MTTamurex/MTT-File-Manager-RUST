use std::sync::Arc;

use crate::index_db;
use crate::indexing_progress::IndexingProgress;
use crate::volume_indices::SharedVolumeIndices;
use crate::volume_lifecycle::VolumeStopSignal;

pub(super) struct FilesystemFallback {
    drive_letter: char,
    volume_id: String,
    volume_serial: u32,
    file_system: String,
    indices: SharedVolumeIndices,
    indexing_progress: Arc<IndexingProgress>,
    db: Arc<index_db::IndexDb>,
    shutdown: VolumeStopSignal,
}

impl FilesystemFallback {
    pub(super) fn new(
        drive_letter: char,
        volume_id: String,
        volume_serial: u32,
        file_system: String,
        indices: SharedVolumeIndices,
        indexing_progress: Arc<IndexingProgress>,
        db: Arc<index_db::IndexDb>,
        shutdown: VolumeStopSignal,
    ) -> Self {
        Self {
            drive_letter,
            volume_id,
            volume_serial,
            file_system,
            indices,
            indexing_progress,
            db,
            shutdown,
        }
    }

    pub(super) fn run(self, reason: &str) {
        if self.shutdown.is_stopped() {
            self.indexing_progress.clear(self.drive_letter);
            return;
        }

        eprintln!(
            "[USN] Volume {} uses filesystem fallback ({})",
            self.drive_letter, reason
        );
        super::super::index_filesystem_fallback(
            self.drive_letter,
            self.volume_id,
            self.volume_serial,
            self.file_system,
            self.indices,
            self.indexing_progress,
            self.db,
            self.shutdown,
        );
    }
}
