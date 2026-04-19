mod non_usn;
mod usn;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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
