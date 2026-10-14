use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::indexing_progress::IndexingProgress;
use crate::usn_journal::DiscoveredVolume;
use crate::volume_indices::{self, SharedVolumeIndices};

mod usage;
mod worker;

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
}

struct VolumeRuntime {
    drive_letter: char,
    volume_id: String,
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
        })
    }

    pub(crate) fn reconcile_discovered_volumes(&self, discovered: &[DiscoveredVolume]) {
        let mut present = HashSet::new();
        for volume in discovered {
            if volume.volume_id.is_empty() {
                eprintln!(
                    "[SERVICE] Skipping indexing for {}:\\ because its volume identity is unavailable",
                    volume.drive_letter
                );
                continue;
            }
            present.insert(volume.drive_letter);
            let _ = self.ensure_runtime(volume);
        }

        let stale: Vec<(char, String)> = self
            .runtimes
            .lock()
            .iter()
            .filter(|(letter, _)| !present.contains(letter))
            .map(|(&letter, runtime)| (letter, runtime.volume_id.clone()))
            .collect();

        for (letter, volume_id) in stale {
            self.disconnect_volume(letter, &volume_id);
        }
    }

    pub(crate) fn claim_worker(&self, volume: &DiscoveredVolume) -> Option<VolumeWorkerTicket> {
        let runtime = self.ensure_runtime(volume)?;
        let mut worker = runtime.worker.lock();
        if runtime.usage.is_removing() || self.global_shutdown.load(Ordering::Acquire) {
            return None;
        }

        if worker.is_some() {
            return None;
        }

        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        let completion = Arc::new(WorkerCompletion::new());
        let stop = VolumeStopSignal::new(self.global_shutdown.clone());
        *worker = Some(WorkerState {
            generation,
            stop: stop.clone(),
            completion: completion.clone(),
        });

        Some(VolumeWorkerTicket {
            runtime: runtime.clone(),
            generation,
            completion,
            stop,
            indexing_progress: self.indexing_progress.clone(),
            discovery_wake: self.discovery_wake.clone(),
        })
    }

    pub(crate) fn prepare_remove(&self, volume_id: &str) -> bool {
        let Some(runtime) = self.runtime_for_id(volume_id) else {
            return true;
        };

        runtime.usage.begin_removal();
        let worker = runtime.worker.lock().clone();
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
            "[PNP] Vetoing removal of {}:\\ while service handles are active (io_idle={}, indexer_stopped={})",
            runtime.drive_letter, usage_idle, worker_finished
        );
        self.remove_index_if_owned(&runtime);
        self.indexing_progress.clear(runtime.drive_letter);
        runtime.usage.cancel_removal();
        let _ = self.discovery_wake.send(());
        false
    }

    pub(crate) fn removal_failed(&self, volume_id: &str) {
        if let Some(runtime) = self.runtime_for_id(volume_id) {
            if runtime.usage.is_removing() {
                self.remove_index_if_owned(&runtime);
                self.indexing_progress.clear(runtime.drive_letter);
            }
            runtime.usage.cancel_removal();
        }
        let _ = self.discovery_wake.send(());
    }

    pub(crate) fn device_arrived(&self) {
        let _ = self.discovery_wake.send(());
    }

    pub(crate) fn disconnect_by_id(&self, volume_id: &str) {
        let letter = self
            .runtimes
            .lock()
            .iter()
            .find(|(_, runtime)| volume_ids_match(&runtime.volume_id, volume_id))
            .map(|(&letter, _)| letter);
        if let Some(letter) = letter {
            self.disconnect_volume(letter, volume_id);
        }
    }

    pub(crate) fn acquire_volume(&self, drive_letter: char) -> Result<Option<VolumeUseGuard>, ()> {
        let runtime = self.runtimes.lock().get(&drive_letter).cloned();
        match runtime {
            Some(runtime) => runtime.usage.acquire().map(Some).ok_or(()),
            None => Ok(None),
        }
    }

    pub(crate) fn acquire_all(&self) -> Result<Vec<VolumeUseGuard>, ()> {
        let runtimes: Vec<_> = self.runtimes.lock().values().cloned().collect();
        let mut guards = Vec::with_capacity(runtimes.len());
        for runtime in runtimes {
            match runtime.usage.acquire() {
                Some(guard) => guards.push(guard),
                None => return Err(()),
            }
        }
        Ok(guards)
    }

    fn ensure_runtime(&self, volume: &DiscoveredVolume) -> Option<Arc<VolumeRuntime>> {
        if volume.volume_id.is_empty() {
            return None;
        }

        loop {
            let existing = self.runtimes.lock().get(&volume.drive_letter).cloned();
            match existing {
                Some(runtime) if volume_ids_match(&runtime.volume_id, &volume.volume_id) => {
                    return Some(runtime);
                }
                Some(runtime) => {
                    runtime.usage.begin_removal();
                    let worker = runtime.worker.lock().clone();
                    if let Some(worker) = &worker {
                        worker.stop.request_stop();
                    }
                    self.remove_index_if_owned(&runtime);
                    let deadline = Instant::now() + REMOVE_WAIT_TIMEOUT;
                    let idle = runtime.usage.wait_until_idle(deadline);
                    let finished = worker
                        .as_ref()
                        .is_none_or(|worker| worker.completion.wait_until(deadline));
                    if !idle || !finished {
                        return None;
                    }

                    let mut runtimes = self.runtimes.lock();
                    if runtimes
                        .get(&volume.drive_letter)
                        .is_some_and(|current| Arc::ptr_eq(current, &runtime))
                    {
                        runtimes.remove(&volume.drive_letter);
                    }
                }
                None => {
                    let runtime = Arc::new(VolumeRuntime {
                        drive_letter: volume.drive_letter,
                        volume_id: volume.volume_id.clone(),
                        usage: Arc::new(VolumeUsage::default()),
                        worker: Mutex::new(None),
                    });
                    let mut runtimes = self.runtimes.lock();
                    match runtimes.get(&volume.drive_letter) {
                        Some(current)
                            if volume_ids_match(&current.volume_id, &volume.volume_id) =>
                        {
                            return Some(current.clone());
                        }
                        Some(_) => continue,
                        None => {
                            runtimes.insert(volume.drive_letter, runtime.clone());
                            return Some(runtime);
                        }
                    }
                }
            }
        }
    }

    fn runtime_for_id(&self, volume_id: &str) -> Option<Arc<VolumeRuntime>> {
        self.runtimes
            .lock()
            .values()
            .find(|runtime| volume_ids_match(&runtime.volume_id, volume_id))
            .cloned()
    }

    fn disconnect_volume(&self, drive_letter: char, volume_id: &str) {
        let runtime = {
            let mut runtimes = self.runtimes.lock();
            let matches = runtimes
                .get(&drive_letter)
                .is_some_and(|runtime| volume_ids_match(&runtime.volume_id, volume_id));
            if matches {
                runtimes.remove(&drive_letter)
            } else {
                None
            }
        };

        if let Some(runtime) = runtime {
            runtime.usage.begin_removal();
            if let Some(worker) = runtime.worker.lock().as_ref() {
                worker.stop.request_stop();
            }
            self.remove_index_if_owned(&runtime);
            self.indexing_progress.clear(drive_letter);
            let _ = self.discovery_wake.send(());
        }
    }

    fn remove_index_if_owned(&self, runtime: &VolumeRuntime) {
        volume_indices::remove_volume_if_identity_matches(
            &self.indices,
            runtime.drive_letter,
            &runtime.volume_id,
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
                *worker = None;
            }
        }
        if self.stop.was_requested() {
            self.indexing_progress.clear(self.runtime.drive_letter);
            let _ = self.discovery_wake.send(());
        }
        self.completion.finish();
    }
}

fn normalize_volume_id(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(['\\', '/'])
        .to_ascii_lowercase()
}

fn volume_ids_match(left: &str, right: &str) -> bool {
    let left = normalize_volume_id(left);
    let right = normalize_volume_id(right);
    if left == right {
        return true;
    }

    let token = |value: &str| {
        let lower = value.to_ascii_lowercase();
        let start = lower.find("volume{")?;
        let rest = &value[start..];
        let end = rest.find('}')? + 1;
        Some(rest[..end].to_ascii_lowercase())
    };
    token(&left).is_some() && token(&left) == token(&right)
}

#[cfg(test)]
mod tests;
