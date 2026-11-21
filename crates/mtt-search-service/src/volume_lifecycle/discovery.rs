use std::collections::HashSet;

use super::*;

impl VolumeLifecycle {
    #[cfg(test)]
    pub(crate) fn reconcile_discovered_volumes(&self, discovered: &[DiscoveredVolume]) {
        // Fixtures provide a current synthetic discovery rather than filesystem I/O.
        self.retire_indexing_drains();
        self.retire_idle_replacements();
        self.reconcile_drive_discovery(discovered, self.drive_config_revision());
    }

    pub(crate) fn reconcile_drive_discovery(
        &self,
        discovered: &[DiscoveredVolume],
        revision: u64,
    ) -> bool {
        self.retire_indexing_drains();
        self.retire_idle_replacements();
        if !self.drive_config.read().matches_revision(revision) {
            return false;
        }
        let mut present = HashSet::new();
        for volume in discovered {
            if volume.volume_id.is_empty() {
                continue;
            }
            present.insert(volume.drive_letter);
            let _ = self.ensure_runtime_for_revision(
                volume,
                Instant::now() + REMOVE_WAIT_TIMEOUT,
                revision,
            );
        }

        let stale: Vec<(char, String, u32)> = {
            let config = self.drive_config.read();
            if !config.matches_revision(revision) {
                return false;
            }
            self.runtimes
                .lock()
                .iter()
                .filter(|(letter, runtime)| {
                    !present.contains(letter) && !runtime.usage.is_removing()
                })
                .map(|(&letter, runtime)| {
                    (letter, runtime.volume_id.clone(), runtime.volume_serial)
                })
                .collect()
        };
        if !self.disconnect_volumes(&stale, Some(revision)) {
            return false;
        }
        self.drive_config.read().matches_revision(revision)
    }

    #[cfg(test)]
    pub(crate) fn claim_worker(
        self: &Arc<Self>,
        volume: &DiscoveredVolume,
    ) -> Option<VolumeWorkerTicket> {
        self.retire_indexing_drains();
        self.claim_discovered_worker(volume, self.drive_config_revision())
    }

    pub(crate) fn claim_discovered_worker(
        self: &Arc<Self>,
        volume: &DiscoveredVolume,
        revision: u64,
    ) -> Option<VolumeWorkerTicket> {
        let runtime = self.ensure_runtime_for_revision(
            volume,
            Instant::now() + REMOVE_WAIT_TIMEOUT,
            revision,
        )?;
        let config = self.drive_config.read();
        if !config.matches_revision(revision)
            || !config.is_enabled(volume.drive_letter)
            || config.draining.contains_key(&volume.drive_letter)
        {
            return None;
        }
        let runtimes = self.runtimes.lock();
        if !runtimes
            .get(&volume.drive_letter)
            .is_some_and(|current| Arc::ptr_eq(current, &runtime))
        {
            return None;
        }
        let mut worker = runtime.worker.lock();
        if runtime.usage.is_removing()
            || self.global_shutdown.load(Ordering::Acquire)
            || worker.is_some()
        {
            return None;
        }

        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        let completion = Arc::new(WorkerCompletion::new());
        let stop = VolumeStopSignal::new(self.global_shutdown.clone()).with_publisher(
            publication::VolumeIndexPublisher::new(self, &runtime, generation),
        );
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
}
