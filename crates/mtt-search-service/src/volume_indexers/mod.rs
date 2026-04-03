mod non_usn;
mod usn;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::file_index;

pub(crate) use non_usn::index_non_ntfs_volume;
pub(crate) use usn::index_volume;

pub(crate) fn wait_for_shutdown_or_timeout(
    shutdown: &Arc<AtomicBool>,
    timeout: std::time::Duration,
) -> bool {
    const STEP: std::time::Duration = std::time::Duration::from_millis(500);
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        if shutdown.load(Ordering::Relaxed) {
            return true;
        }

        let remaining = timeout.saturating_sub(start.elapsed());
        std::thread::sleep(STEP.min(remaining));
    }

    shutdown.load(Ordering::Relaxed)
}

pub(super) fn upsert_volume_index(
    indices: &mut Vec<file_index::VolumeIndex>,
    new_index: file_index::VolumeIndex,
) {
    if let Some(existing) = indices
        .iter_mut()
        .find(|v| v.drive_letter == new_index.drive_letter)
    {
        *existing = new_index;
    } else {
        indices.push(new_index);
    }
}
