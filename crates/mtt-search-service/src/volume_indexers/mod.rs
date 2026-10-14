mod dispatcher;
mod non_usn;
mod usn;

use std::sync::OnceLock;

pub(crate) use dispatcher::spawn_volume_indexer;
pub(crate) use non_usn::index_filesystem_fallback;
pub(crate) use usn::index_volume;

use crate::volume_lifecycle::VolumeStopSignal;

fn index_build_lock() -> &'static parking_lot::Mutex<()> {
    static LOCK: OnceLock<parking_lot::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| parking_lot::Mutex::new(()))
}

fn binary_snapshot_lock() -> &'static parking_lot::Mutex<()> {
    static LOCK: OnceLock<parking_lot::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| parking_lot::Mutex::new(()))
}

pub(crate) fn try_begin_index_build() -> Option<parking_lot::MutexGuard<'static, ()>> {
    index_build_lock().try_lock()
}

pub(crate) fn wait_for_index_build(
    stop: &VolumeStopSignal,
) -> Option<parking_lot::MutexGuard<'static, ()>> {
    loop {
        if stop.is_stopped() {
            return None;
        }
        if let Some(guard) = try_begin_index_build() {
            return Some(guard);
        }
        stop.wait_or_stop(std::time::Duration::from_millis(50));
    }
}

/// A checkpoint is streamed, but it still walks the complete mapped index.
pub(crate) fn begin_binary_snapshot() -> parking_lot::MutexGuard<'static, ()> {
    binary_snapshot_lock().lock()
}

pub(crate) fn wait_for_shutdown_or_timeout(
    shutdown: &VolumeStopSignal,
    timeout: std::time::Duration,
) -> bool {
    const SHUTDOWN_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        if shutdown.is_stopped() {
            return true;
        }

        let remaining = timeout.saturating_sub(start.elapsed());
        if shutdown.wait_or_stop(remaining.min(SHUTDOWN_POLL_INTERVAL)) {
            return true;
        }
    }

    shutdown.is_stopped()
}
