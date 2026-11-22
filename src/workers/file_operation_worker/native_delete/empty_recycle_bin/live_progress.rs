//! Observe actual removals even when Shell omits usable per-item callbacks.
use super::{reconciliation::Snapshot, tracking::Tracking};
use crate::infrastructure::windows::recycle_bin::ComApartmentGuard;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, RecvTimeoutError, Sender},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const INTERVAL: Duration = Duration::from_millis(200);

pub(super) struct LiveProgress {
    stopped: Arc<AtomicBool>,
    wake: Sender<()>,
    worker: Option<JoinHandle<()>>,
}

impl LiveProgress {
    pub(super) fn start(snapshot: Snapshot, tracking: Arc<Tracking>) -> std::io::Result<Self> {
        Self::start_with(move |stopped| {
            // Shell objects are created and dropped on this observer's STA;
            // only cached identity strings and atomic counters cross threads.
            snapshot.reconcile_until(&tracking, || stopped.load(Ordering::Acquire));
        })
    }

    fn start_with(
        mut reconcile: impl FnMut(&AtomicBool) + Send + 'static,
    ) -> std::io::Result<Self> {
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = stopped.clone();
        let (wake, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("recycle-bin-progress".into())
            .spawn(move || {
                let _com = ComApartmentGuard::init_sta_best_effort();
                loop {
                    match receiver.recv_timeout(INTERVAL) {
                        Err(RecvTimeoutError::Timeout) => {
                            if worker_stopped.load(Ordering::Acquire) {
                                break;
                            }
                            reconcile(&worker_stopped);
                        }
                        Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            })?;
        Ok(Self {
            stopped,
            wake,
            worker: Some(worker),
        })
    }
}

impl Drop for LiveProgress {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let _ = self.wake.send(());
        if let Some(worker) = self.worker.take() {
            // Join before final reconciliation: a stale observation must never
            // overwrite final counts or outlive a cancelled/failed operation.
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests;
