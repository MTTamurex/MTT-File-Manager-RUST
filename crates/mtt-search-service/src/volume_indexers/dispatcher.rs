use std::sync::Arc;

use crate::index_db;
use crate::indexing_progress::IndexingProgress;
use crate::usn_journal::DiscoveredVolume;
use crate::volume_indices::SharedVolumeIndices;
use crate::volume_lifecycle::VolumeLifecycle;

pub(crate) fn spawn_volume_indexer(
    volume: DiscoveredVolume,
    lifecycle: &Arc<VolumeLifecycle>,
    indices: SharedVolumeIndices,
    indexing_progress: Arc<IndexingProgress>,
    db: Arc<index_db::IndexDb>,
) {
    let Some(ticket) = lifecycle.claim_worker(&volume) else {
        return;
    };

    let drive_letter = volume.drive_letter;
    let stop = ticket.stop_signal();
    let label = if volume.label.is_empty() {
        "(no label)"
    } else {
        volume.label.as_str()
    };

    if volume.usn_supported {
        eprintln!(
            "[SERVICE] Starting USN indexer for {}:\\ ({}, {})",
            drive_letter, label, volume.file_system
        );
    } else {
        eprintln!(
            "[SERVICE] Starting fallback scanner for {}:\\ ({}, {})",
            drive_letter, label, volume.file_system
        );
    }

    indexing_progress.set_pending(drive_letter);
    let progress_on_spawn_failure = indexing_progress.clone();
    let spawn_result = std::thread::Builder::new()
        .name(format!("volume-index-{drive_letter}"))
        .spawn(move || {
            let _ticket = ticket;
            let usn_supported = volume.usn_supported;
            let volume = super::IndexVolume {
                drive_letter,
                volume_id: volume.volume_id,
                volume_serial: volume.volume_serial,
                file_system: volume.file_system,
            };
            if usn_supported {
                super::index_volume(volume, indices, indexing_progress, db, stop);
            } else {
                super::index_filesystem_fallback(volume, indices, indexing_progress, db, stop);
            }
        });
    if let Err(error) = spawn_result {
        progress_on_spawn_failure.clear(drive_letter);
        eprintln!(
            "[SERVICE] Failed to spawn indexer for {}:\\: {}",
            drive_letter, error
        );
    }
}
