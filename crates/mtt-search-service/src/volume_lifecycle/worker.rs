use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Condvar, Mutex};

pub(super) struct WorkerCompletion {
    finished: Mutex<bool>,
    changed: Condvar,
}

#[derive(Clone)]
pub(crate) struct VolumeStopSignal {
    global_shutdown: Arc<AtomicBool>,
    local_stop: Arc<LocalStopState>,
    publisher: Option<Arc<super::publication::VolumeIndexPublisher>>,
}

struct LocalStopState {
    requested: AtomicBool,
    wait_lock: Mutex<()>,
    wake: Condvar,
}

impl VolumeStopSignal {
    pub(super) fn new(global_shutdown: Arc<AtomicBool>) -> Self {
        Self {
            global_shutdown,
            local_stop: Arc::new(LocalStopState {
                requested: AtomicBool::new(false),
                wait_lock: Mutex::new(()),
                wake: Condvar::new(),
            }),
            publisher: None,
        }
    }

    pub(super) fn with_publisher(
        mut self,
        publisher: super::publication::VolumeIndexPublisher,
    ) -> Self {
        self.publisher = Some(Arc::new(publisher));
        self
    }

    pub(crate) fn publish_index(
        &self,
        indices: &crate::volume_indices::SharedVolumeIndices,
        index: crate::file_index::VolumeIndex,
    ) -> Option<crate::volume_indices::VolumeIndexHandle> {
        self.publisher.as_ref()?.publish(indices, index)
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.global_shutdown.load(Ordering::Acquire)
            || self.local_stop.requested.load(Ordering::Acquire)
    }

    pub(crate) fn wait_or_stop(&self, timeout: Duration) -> bool {
        if self.is_stopped() {
            return true;
        }
        let mut guard = self.local_stop.wait_lock.lock();
        if !self.is_stopped() {
            self.local_stop.wake.wait_for(&mut guard, timeout);
        }
        self.is_stopped()
    }

    pub(super) fn request_stop(&self) {
        let _guard = self.local_stop.wait_lock.lock();
        self.local_stop.requested.store(true, Ordering::Release);
        self.local_stop.wake.notify_all();
    }

    pub(crate) fn was_requested(&self) -> bool {
        self.local_stop.requested.load(Ordering::Acquire)
    }
}

impl WorkerCompletion {
    pub(super) fn new() -> Self {
        Self {
            finished: Mutex::new(false),
            changed: Condvar::new(),
        }
    }

    pub(super) fn finish(&self) {
        *self.finished.lock() = true;
        self.changed.notify_all();
    }

    pub(super) fn wait_until(&self, deadline: std::time::Instant) -> bool {
        let mut finished = self.finished.lock();
        while !*finished {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return false;
            }
            self.changed.wait_for(&mut finished, remaining);
        }
        true
    }
}
