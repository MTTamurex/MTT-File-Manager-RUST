use super::*;

impl VolumeLifecycle {
    #[cfg(test)]
    pub(super) fn ensure_runtime_until(
        &self,
        volume: &DiscoveredVolume,
        deadline: Instant,
    ) -> Option<Arc<VolumeRuntime>> {
        self.retire_indexing_drains();
        self.ensure_runtime_for_revision(volume, deadline, self.drive_config_revision())
    }

    pub(super) fn ensure_runtime_for_revision(
        &self,
        volume: &DiscoveredVolume,
        deadline: Instant,
        revision: u64,
    ) -> Option<Arc<VolumeRuntime>> {
        if volume.volume_id.is_empty() {
            return None;
        }

        loop {
            self.retire_indexing_drains();
            let (runtime, worker) = {
                let config = self.drive_config.read();
                if !config.matches_revision(revision)
                    || !config.is_enabled(volume.drive_letter)
                    || config.draining.contains_key(&volume.drive_letter)
                {
                    return None;
                }
                // Publication and invalidation always take registry -> worker.
                let mut runtimes = self.runtimes.lock();
                let Some(runtime) = runtimes.get(&volume.drive_letter).cloned() else {
                    let runtime = Arc::new(VolumeRuntime {
                        drive_letter: volume.drive_letter,
                        volume_id: volume.volume_id.clone(),
                        volume_serial: volume.volume_serial,
                        usage: Arc::new(VolumeUsage::new(self.discovery_wake.clone())),
                        worker: Mutex::new(None),
                    });
                    runtimes.insert(volume.drive_letter, runtime.clone());
                    return Some(runtime);
                };
                if volume_ids_match(&runtime.volume_id, &volume.volume_id)
                    && runtime.volume_serial == volume.volume_serial
                {
                    return Some(runtime);
                }
                let worker = {
                    let worker = runtime.worker.lock();
                    runtime.usage.begin_replacement();
                    if let Some(worker) = worker.as_ref() {
                        worker.stop.request_stop();
                    }
                    self.remove_index_if_owned(&runtime);
                    worker.clone()
                };
                (runtime, worker)
            };

            let idle = runtime.usage.wait_until_idle(deadline);
            let finished = worker
                .as_ref()
                .is_none_or(|worker| worker.completion.wait_until(deadline));
            if !idle || !finished {
                // Keep the old identity closed. Reconciliation retires it once
                // drained and wakes discovery to obtain the current identity.
                return None;
            }
            let config = self.drive_config.read();
            if !config.matches_revision(revision) {
                return None;
            }
            let mut runtimes = self.runtimes.lock();
            if !runtime.usage.replacement_is_idle() {
                return None; // A PnP maintenance/removal request still owns it.
            }
            if runtimes
                .get(&volume.drive_letter)
                .is_some_and(|current| Arc::ptr_eq(current, &runtime))
            {
                runtimes.remove(&volume.drive_letter);
            }
        }
    }

    pub(super) fn retire_idle_replacements(&self) {
        let mut config = self.drive_config.write();
        let mut runtimes = self.runtimes.lock();
        let before = runtimes.len();
        runtimes.retain(|_, runtime| {
            let worker = runtime.worker.lock();
            !runtime.usage.replacement_is_idle() || worker.is_some()
        });
        if runtimes.len() != before {
            // This pass may have skipped the suspended letter before retirement.
            // Only a subsequent discovery can attest to complete coverage.
            config.invalidate_discovery();
            let _ = self.discovery_wake.send(());
        }
    }
}
