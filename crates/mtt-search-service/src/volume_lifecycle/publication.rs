use std::sync::{Arc, Weak};

use crate::file_index::VolumeIndex;
use crate::volume_indices::{self, SharedVolumeIndices, VolumeIndexHandle};

use super::{volume_ids_match, VolumeLifecycle, VolumeRuntime};

pub(super) struct VolumeIndexPublisher {
    lifecycle: Weak<VolumeLifecycle>,
    runtime: Weak<VolumeRuntime>,
    generation: u64,
}

impl VolumeIndexPublisher {
    pub(super) fn new(
        lifecycle: &Arc<VolumeLifecycle>,
        runtime: &Arc<VolumeRuntime>,
        generation: u64,
    ) -> Self {
        Self {
            lifecycle: Arc::downgrade(lifecycle),
            runtime: Arc::downgrade(runtime),
            generation,
        }
    }

    pub(super) fn publish(
        &self,
        indices: &SharedVolumeIndices,
        index: VolumeIndex,
    ) -> Option<VolumeIndexHandle> {
        let lifecycle = self.lifecycle.upgrade()?;
        let runtime = self.runtime.upgrade()?;
        let runtimes = lifecycle.runtimes.lock();
        let current = runtimes.get(&index.drive_letter)?;
        if !Arc::ptr_eq(current, &runtime)
            || !Arc::ptr_eq(indices, &lifecycle.indices)
            || !volume_ids_match(&index.volume_id, &runtime.volume_id)
            || index.volume_serial != runtime.volume_serial
        {
            return None;
        }
        let worker = runtime.worker.lock();
        let worker = worker.as_ref()?;
        if worker.generation != self.generation
            || worker.stop.is_stopped()
            || runtime.usage.is_removing()
        {
            return None;
        }
        // Keep both locks through upsert: disconnect cannot invalidate the
        // runtime between the ownership check and publication.
        Some(volume_indices::upsert(indices, index))
    }
}
