mod non_usn;
mod usn;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

pub(crate) use non_usn::index_non_ntfs_volume;
pub(crate) use usn::index_volume;

fn index_build_lock() -> &'static parking_lot::Mutex<()> {
    static LOCK: OnceLock<parking_lot::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| parking_lot::Mutex::new(()))
}

fn binary_snapshot_lock() -> &'static parking_lot::Mutex<()> {
    static LOCK: OnceLock<parking_lot::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| parking_lot::Mutex::new(()))
}

/// Keep full-volume builders from multiplying their O(N) working sets across drives.
pub(crate) fn begin_index_build() -> parking_lot::MutexGuard<'static, ()> {
    index_build_lock().lock()
}

pub(crate) fn try_begin_index_build() -> Option<parking_lot::MutexGuard<'static, ()>> {
    index_build_lock().try_lock()
}

/// A checkpoint is streamed, but it still walks the complete mapped index.
pub(crate) fn begin_binary_snapshot() -> parking_lot::MutexGuard<'static, ()> {
    binary_snapshot_lock().lock()
}

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
