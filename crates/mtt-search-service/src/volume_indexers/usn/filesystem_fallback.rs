use std::sync::Arc;

use crate::index_db;
use crate::indexing_progress::IndexingProgress;
use crate::volume_indices::SharedVolumeIndices;
use crate::volume_lifecycle::VolumeStopSignal;

pub(super) struct FilesystemFallback {
    volume: super::super::IndexVolume,
    indices: SharedVolumeIndices,
    indexing_progress: Arc<IndexingProgress>,
    db: Arc<index_db::IndexDb>,
    shutdown: VolumeStopSignal,
}

impl FilesystemFallback {
    pub(super) fn new(
        volume: super::super::IndexVolume,
        indices: SharedVolumeIndices,
        indexing_progress: Arc<IndexingProgress>,
        db: Arc<index_db::IndexDb>,
        shutdown: VolumeStopSignal,
    ) -> Self {
        Self {
            volume,
            indices,
            indexing_progress,
            db,
            shutdown,
        }
    }

    pub(super) fn run(self, reason: &str) {
        if self.shutdown.is_stopped() {
            self.indexing_progress.clear(self.volume.drive_letter);
            return;
        }

        eprintln!(
            "[USN] Volume {} uses filesystem fallback ({})",
            self.volume.drive_letter, reason
        );
        super::super::index_filesystem_fallback(
            self.volume,
            self.indices,
            self.indexing_progress,
            self.db,
            self.shutdown,
        );
    }
}
