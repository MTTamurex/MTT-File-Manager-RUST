//! Serialized configuration commands and bounded, coalesced status publication.
//! A write with a lost acknowledgement is reconciled, never replayed.
use crate::infrastructure::drive_indexing::{
    DriveIndexingError, DriveIndexingErrorKind, DriveIndexingSnapshot, SharedDriveIndexing,
};
use crossbeam_channel::{bounded, Receiver, Sender};
use parking_lot::RwLock;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const REFRESH_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub struct DriveIndexingChange {
    pub request_id: u64,
    pub drive_letter: char,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveIndexingWriteOutcome {
    Queued,
    Pending,
    Confirmed { enabled: bool },
    Failed(DriveIndexingError),
}

#[derive(Debug, Clone)]
pub struct DriveIndexingOperation {
    pub request_id: u64,
    pub requested_enabled: bool,
    pub outcome: DriveIndexingWriteOutcome,
}

#[derive(Debug, Clone, Default)]
pub struct DriveIndexingWorkerStatus {
    // At most one latest operation per ASCII drive, not an accumulating event queue.
    pub operations: BTreeMap<char, DriveIndexingOperation>,
    pub refresh_error: Option<DriveIndexingError>,
}

enum Request {
    Refresh,
    Set(DriveIndexingChange),
}

pub struct DriveIndexingWorker {
    requests: Sender<Request>,
    refresh_queued: Arc<AtomicBool>,
    status: Arc<RwLock<DriveIndexingWorkerStatus>>,
    running: Arc<AtomicBool>,
}

impl DriveIndexingWorker {
    pub fn start(policy: SharedDriveIndexing, ctx: eframe::egui::Context) -> std::io::Result<Self> {
        let (requests, receiver) = bounded(32);
        let status = Arc::new(RwLock::new(DriveIndexingWorkerStatus::default()));
        let refresh_queued = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));
        let lifetime = Lifetime(running.clone());
        let mut core = WorkerCore {
            policy,
            status: status.clone(),
        };
        let queued = refresh_queued.clone();
        std::thread::Builder::new()
            .name("drive-indexing-control".into())
            .spawn(move || {
                let _lifetime = lifetime;
                run(&mut core, &mut ServiceBackend, receiver, queued, ctx);
            })?;
        Ok(Self {
            requests,
            refresh_queued,
            status,
            running,
        })
    }

    pub fn status(&self) -> DriveIndexingWorkerStatus {
        self.status.read().clone()
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    pub fn refresh(&self) -> Result<(), DriveIndexingError> {
        if !self.is_running() {
            return Err(DriveIndexingError::transport(
                "Drive configuration worker stopped",
            ));
        }
        if self.refresh_queued.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        if self.requests.try_send(Request::Refresh).is_err() {
            self.refresh_queued.store(false, Ordering::Release);
            return Err(DriveIndexingError::new(
                DriveIndexingErrorKind::Busy,
                "Drive configuration queue unavailable",
            ));
        }
        Ok(())
    }

    pub fn set(&self, mut change: DriveIndexingChange) -> Result<(), DriveIndexingError> {
        if !change.drive_letter.is_ascii_alphabetic() {
            return Err(DriveIndexingError::new(
                DriveIndexingErrorKind::InvalidDrive,
                "Invalid drive letter",
            ));
        }
        change.drive_letter = change.drive_letter.to_ascii_uppercase();
        if !self.is_running() {
            return Err(DriveIndexingError::transport(
                "Drive configuration worker stopped",
            ));
        }
        let mut status = self.status.write();
        if status
            .operations
            .get(&change.drive_letter)
            .is_some_and(|operation| {
                matches!(
                    operation.outcome,
                    DriveIndexingWriteOutcome::Queued | DriveIndexingWriteOutcome::Pending
                )
            })
        {
            return Err(DriveIndexingError::new(
                DriveIndexingErrorKind::Busy,
                "Drive configuration change is still pending",
            ));
        }
        let letter = change.drive_letter;
        let previous = status.operations.insert(
            letter,
            DriveIndexingOperation {
                request_id: change.request_id,
                requested_enabled: change.enabled,
                outcome: DriveIndexingWriteOutcome::Queued,
            },
        );
        if self.requests.try_send(Request::Set(change)).is_err() {
            if let Some(previous) = previous {
                status.operations.insert(letter, previous);
            } else {
                status.operations.remove(&letter);
            }
            return Err(DriveIndexingError::new(
                DriveIndexingErrorKind::Busy,
                "Drive configuration queue unavailable",
            ));
        }
        Ok(())
    }
}

struct Lifetime(Arc<AtomicBool>);
impl Drop for Lifetime {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

trait Backend {
    fn get(&mut self) -> Result<DriveIndexingSnapshot, DriveIndexingError>;
    fn set(&mut self, letter: char, enabled: bool) -> Result<(), DriveIndexingError>;
}

struct ServiceBackend;
impl Backend for ServiceBackend {
    fn get(&mut self) -> Result<DriveIndexingSnapshot, DriveIndexingError> {
        crate::infrastructure::global_search::get_drive_indexing_config()
    }
    fn set(&mut self, letter: char, enabled: bool) -> Result<(), DriveIndexingError> {
        crate::infrastructure::global_search::set_drive_indexing(letter, enabled)
    }
}

struct WorkerCore {
    policy: SharedDriveIndexing,
    status: Arc<RwLock<DriveIndexingWorkerStatus>>,
}

impl WorkerCore {
    fn refresh(&mut self, backend: &mut impl Backend) {
        match backend.get() {
            Ok(snapshot) => {
                self.policy.confirm(snapshot);
                let view = self.policy.snapshot();
                let mut status = self.status.write();
                status.refresh_error = None;
                for (&letter, operation) in &mut status.operations {
                    if operation.outcome == DriveIndexingWriteOutcome::Pending {
                        operation.outcome = DriveIndexingWriteOutcome::Confirmed {
                            enabled: view.is_enabled(letter),
                        };
                    }
                }
            }
            Err(error) => {
                self.policy.unavailable(&error);
                let mut status = self.status.write();
                if error.kind == DriveIndexingErrorKind::Unsupported {
                    for operation in status.operations.values_mut() {
                        if operation.outcome == DriveIndexingWriteOutcome::Pending {
                            operation.outcome = DriveIndexingWriteOutcome::Failed(error.clone());
                        }
                    }
                }
                status.refresh_error = Some(error);
            }
        }
    }

    /// Returns whether a GET was attempted (and thus resets periodic scheduling).
    fn set(&mut self, change: DriveIndexingChange, backend: &mut impl Backend) -> bool {
        let outcome = match backend.set(change.drive_letter, change.enabled) {
            Ok(()) => DriveIndexingWriteOutcome::Pending,
            Err(error) if error.may_have_applied() => DriveIndexingWriteOutcome::Pending,
            Err(error) => {
                if matches!(error.kind, DriveIndexingErrorKind::AccessDenied) {
                    self.policy.unavailable(&error);
                }
                DriveIndexingWriteOutcome::Failed(error)
            }
        };
        let reconcile = outcome == DriveIndexingWriteOutcome::Pending;
        self.status.write().operations.insert(
            change.drive_letter,
            DriveIndexingOperation {
                request_id: change.request_id,
                requested_enabled: change.enabled,
                outcome,
            },
        );
        if reconcile {
            self.refresh(backend);
        }
        reconcile
    }
}

fn run(
    core: &mut WorkerCore,
    backend: &mut impl Backend,
    receiver: Receiver<Request>,
    queued: Arc<AtomicBool>,
    ctx: eframe::egui::Context,
) {
    core.refresh(backend);
    ctx.request_repaint();
    let mut next_refresh = Instant::now() + REFRESH_INTERVAL;
    loop {
        if Instant::now() >= next_refresh {
            core.refresh(backend);
            ctx.request_repaint();
            next_refresh = Instant::now() + REFRESH_INTERVAL;
        }
        let request =
            match receiver.recv_timeout(next_refresh.saturating_duration_since(Instant::now())) {
                Ok(request) => request,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            };
        let refreshed = match request {
            Request::Refresh => {
                queued.store(false, Ordering::Release);
                core.refresh(backend);
                true
            }
            Request::Set(change) => core.set(change, backend),
        };
        if refreshed {
            next_refresh = Instant::now() + REFRESH_INTERVAL;
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests;
