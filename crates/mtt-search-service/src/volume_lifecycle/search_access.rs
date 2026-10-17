use crate::volume_indices::{self, VolumeIndexHandle};

use super::{volume_ids_match, VolumeLifecycle, VolumeUseGuard};

/// A request searches only these handles, keeping their I/O leases until all
/// filesystem authorization checks have finished. Suspended volumes are absent.
pub(crate) struct VolumeSearchAccess {
    handles: Vec<VolumeIndexHandle>,
    _guards: Vec<VolumeUseGuard>,
}

impl VolumeSearchAccess {
    pub(crate) fn handles(&self) -> &[VolumeIndexHandle] {
        &self.handles
    }

    pub(crate) fn find_handle(&self, drive_letter: char) -> Option<&VolumeIndexHandle> {
        self.handles
            .iter()
            .find(|handle| handle.read().drive_letter == drive_letter)
    }
}

impl VolumeLifecycle {
    pub(crate) fn acquire_search_access(&self) -> super::VolumeSearchAccess {
        let runtimes = self.runtimes.lock();
        let mut handles = Vec::new();
        let mut guards = Vec::new();
        for handle in volume_indices::snapshot_handles(&self.indices) {
            let index = handle.read();
            let Some(runtime) = runtimes.get(&index.drive_letter) else {
                continue;
            };
            if !volume_ids_match(&runtime.volume_id, &index.volume_id)
                || runtime.volume_serial != index.volume_serial
            {
                continue;
            }
            let Some(guard) = runtime.usage.acquire() else {
                continue;
            };
            drop(index);
            handles.push(handle);
            guards.push(guard);
        }
        VolumeSearchAccess {
            handles,
            _guards: guards,
        }
    }
}
