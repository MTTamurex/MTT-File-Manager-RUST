use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};

use crate::indexing_progress::IndexingProgress;
use crate::usn_journal::DiscoveredVolume;
use crate::volume_indices::{self, SharedVolumeIndices};

mod discovery;
mod identity;
mod indexing_config;
mod publication;
mod runtime;
mod search_access;
mod usage;
mod worker;

#[cfg(test)]
use identity::normalize_volume_id;
use identity::volume_ids_match;
pub(crate) use search_access::VolumeSearchAccess;
use usage::VolumeUsage;
pub(crate) use usage::VolumeUseGuard;
pub(crate) use worker::VolumeStopSignal;
use worker::WorkerCompletion;

const REMOVE_WAIT_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct VolumeLifecycle {
    global_shutdown: Arc<AtomicBool>,
    indices: SharedVolumeIndices,
    indexing_progress: Arc<IndexingProgress>,
    discovery_wake: Sender<()>,
    next_generation: AtomicU64,
    runtimes: Mutex<HashMap<char, Arc<VolumeRuntime>>>,
    // Lock order: configuration -> runtime registry -> worker -> indices.
    // The update mutex serializes persistence but is never used by indexers.
    drive_config: RwLock<indexing_config::DriveConfigState>,
    config_updates: Mutex<()>,
    config_db: std::sync::OnceLock<Arc<crate::index_db::IndexDb>>,
}

struct VolumeRuntime {
    drive_letter: char,
    volume_id: String,
    volume_serial: u32,
    usage: Arc<VolumeUsage>,
    worker: Mutex<Option<WorkerState>>,
}

#[derive(Clone)]
struct WorkerState {
    generation: u64,
    stop: VolumeStopSignal,
    completion: Arc<WorkerCompletion>,
}

pub(crate) struct VolumeWorkerTicket {
    runtime: Arc<VolumeRuntime>,
    generation: u64,
    completion: Arc<WorkerCompletion>,
    stop: VolumeStopSignal,
    indexing_progress: Arc<IndexingProgress>,
    discovery_wake: Sender<()>,
}

impl VolumeLifecycle {
    pub(crate) fn new(
        global_shutdown: Arc<AtomicBool>,
        indices: SharedVolumeIndices,
        indexing_progress: Arc<IndexingProgress>,
        discovery_wake: Sender<()>,
    ) -> Arc<Self> {
        Arc::new(Self {
            global_shutdown,
            indices,
            indexing_progress,
            discovery_wake,
            next_generation: AtomicU64::new(1),
            runtimes: Mutex::new(HashMap::new()),
            drive_config: RwLock::new(indexing_config::DriveConfigState::default()),
            config_updates: Mutex::new(()),
            config_db: std::sync::OnceLock::new(),
        })
    }

    pub(crate) fn prepare_remove_identity(&self, volume_id: &str, volume_serial: u32) -> bool {
        self.prepare_operation(volume_id, volume_serial, true)
    }

    /// Maintenance notifications cannot veto the operation. Keep the volume
    /// suspended even if draining existing work exceeds the callback budget.
    pub(crate) fn prepare_maintenance_identity(&self, volume_id: &str, volume_serial: u32) -> bool {
        self.prepare_operation(volume_id, volume_serial, false)
    }

    fn prepare_operation(
        &self,
        volume_id: &str,
        volume_serial: u32,
        restore_on_timeout: bool,
    ) -> bool {
        let Some((runtime, worker)) = self.begin_identity_removal(volume_id, volume_serial) else {
            return true;
        };
        if let Some(worker) = &worker {
            worker.stop.request_stop();
        }

        let deadline = Instant::now() + REMOVE_WAIT_TIMEOUT;
        let usage_idle = runtime.usage.wait_until_idle(deadline);
        let worker_finished = worker
            .as_ref()
            .is_none_or(|worker| worker.completion.wait_until(deadline));

        if usage_idle && worker_finished {
            return true;
        }

        eprintln!(
            "[PNP] Service handles remain active for {}:\\ (io_idle={}, indexer_stopped={})",
            runtime.drive_letter, usage_idle, worker_finished
        );
        {
            let _config = self.drive_config.read();
            let runtimes = self.runtimes.lock();
            // A detached identity must never clear a reconnected generation.
            if runtimes
                .get(&runtime.drive_letter)
                .is_some_and(|current| Arc::ptr_eq(current, &runtime))
            {
                self.remove_index_if_owned(&runtime);
                self.indexing_progress.clear(runtime.drive_letter);
            }
            if restore_on_timeout {
                runtime.usage.cancel_removal();
            }
        }
        if restore_on_timeout {
            self.wake_discovery();
        }
        false
    }

    pub(crate) fn removal_failed_identity(&self, volume_id: &str, volume_serial: u32) {
        {
            let mut config = self.drive_config.write();
            let runtimes = self.runtimes.lock();
            if let Some(runtime) = config.runtime_for_identity(&runtimes, volume_id, volume_serial)
            {
                if runtime.usage.is_removing()
                    && runtimes
                        .get(&runtime.drive_letter)
                        .is_some_and(|current| Arc::ptr_eq(current, &runtime))
                {
                    self.remove_index_if_owned(&runtime);
                    self.indexing_progress.clear(runtime.drive_letter);
                }
                runtime.usage.cancel_removal();
                config.invalidate_discovery();
            }
        }
        self.wake_discovery();
    }

    pub(crate) fn is_removing(&self, volume_id: &str, volume_serial: u32) -> bool {
        let config = self.drive_config.read();
        let runtimes = self.runtimes.lock();
        config
            .runtime_for_identity(&runtimes, volume_id, volume_serial)
            .is_some_and(|runtime| {
                runtime.usage.is_removing() || config.draining.contains_key(&runtime.drive_letter)
            })
    }

    pub(crate) fn is_drive_suspended(&self, letter: char) -> bool {
        let config = self.drive_config.read();
        config.draining.contains_key(&letter)
            || self
                .runtimes
                .lock()
                .get(&letter)
                .is_some_and(|runtime| runtime.usage.is_removing())
    }

    pub(crate) fn wake_discovery(&self) {
        let _ = self.discovery_wake.send(());
    }

    pub(crate) fn disconnect_by_identity(&self, volume_id: &str, volume_serial: u32) {
        if let Some(runtime) = self.runtime_for_identity(volume_id, volume_serial) {
            self.disconnect_volumes(
                &[(runtime.drive_letter, volume_id.to_string(), volume_serial)],
                None,
            );
        }
    }

    pub(crate) fn acquire_volume(&self, drive_letter: char) -> Result<Option<VolumeUseGuard>, ()> {
        let config = self.drive_config.read();
        if !config.is_enabled(drive_letter) {
            return Err(());
        }
        let runtimes = self.runtimes.lock();
        match runtimes.get(&drive_letter) {
            Some(runtime) => runtime.usage.acquire().map(Some).ok_or(()),
            None if config.draining.contains_key(&drive_letter) => Err(()),
            None => Ok(None),
        }
    }

    pub(crate) fn acquire_hardware_volume(
        &self,
        drive_letter: char,
    ) -> Result<Option<VolumeUseGuard>, ()> {
        let config = self.drive_config.read();
        let runtimes = self.runtimes.lock();
        match runtimes.get(&drive_letter) {
            Some(runtime) => runtime.usage.acquire_hardware().map(Some).ok_or(()),
            None if config.draining.contains_key(&drive_letter) => Err(()),
            None => Ok(None),
        }
    }

    fn runtime_for_identity(
        &self,
        volume_id: &str,
        volume_serial: u32,
    ) -> Option<Arc<VolumeRuntime>> {
        let config = self.drive_config.read();
        let runtimes = self.runtimes.lock();
        config.runtime_for_identity(&runtimes, volume_id, volume_serial)
    }

    fn begin_identity_removal(
        &self,
        volume_id: &str,
        volume_serial: u32,
    ) -> Option<(Arc<VolumeRuntime>, Option<WorkerState>)> {
        let mut config = self.drive_config.write();
        let runtimes = self.runtimes.lock();
        let runtime = config.runtime_for_identity(&runtimes, volume_id, volume_serial)?;
        let worker = {
            // Pin the registry through the transition: retirement cannot replace
            // this identity between lookup and suspension.
            let worker = runtime.worker.lock();
            runtime.usage.begin_removal();
            worker.clone()
        };
        config.invalidate_discovery();
        self.wake_discovery();
        Some((runtime, worker))
    }

    fn disconnect_volumes(&self, volumes: &[(char, String, u32)], revision: Option<u64>) -> bool {
        let disconnected = {
            let mut config = self.drive_config.write();
            if revision.is_some_and(|revision| !config.matches_revision(revision)) {
                return false;
            }
            let mut runtimes = self.runtimes.lock();
            let mut disconnected = false;
            for (drive_letter, volume_id, volume_serial) in volumes {
                let matches = runtimes.get(drive_letter).is_some_and(|runtime| {
                    volume_ids_match(&runtime.volume_id, volume_id)
                        && runtime.volume_serial == *volume_serial
                });
                if matches {
                    let runtime = runtimes.remove(drive_letter).unwrap();
                    let worker = runtime.worker.lock();
                    runtime.usage.begin_removal();
                    if let Some(worker) = worker.as_ref() {
                        worker.stop.request_stop();
                    }
                    config.draining.insert(*drive_letter, runtime.clone());
                    self.remove_index_if_owned(&runtime);
                    self.indexing_progress.clear(*drive_letter);
                    disconnected = true;
                }
            }
            if disconnected {
                config.invalidate_discovery();
            }
            disconnected
        };

        if disconnected {
            let _ = self.discovery_wake.send(());
        }
        true
    }

    fn remove_index_if_owned(&self, runtime: &VolumeRuntime) {
        volume_indices::remove_volume_if_identity_matches(
            &self.indices,
            runtime.drive_letter,
            &runtime.volume_id,
            runtime.volume_serial,
        );
    }
}

impl VolumeWorkerTicket {
    pub(crate) fn stop_signal(&self) -> VolumeStopSignal {
        self.stop.clone()
    }
}

impl Drop for VolumeWorkerTicket {
    fn drop(&mut self) {
        {
            let mut worker = self.runtime.worker.lock();
            if worker
                .as_ref()
                .is_some_and(|state| state.generation == self.generation)
            {
                // Finish cleanup before making this generation replaceable.
                if self.stop.was_requested() {
                    self.indexing_progress.clear(self.runtime.drive_letter);
                }
                self.completion.finish();
                *worker = None;
            }
        }
        if self.stop.was_requested() {
            let _ = self.discovery_wake.send(());
        }
    }
}

#[cfg(test)]
mod tests;
