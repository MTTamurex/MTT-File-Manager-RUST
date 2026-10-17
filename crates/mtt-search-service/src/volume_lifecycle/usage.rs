use std::sync::Arc;

use parking_lot::{Condvar, Mutex};

#[derive(Default)]
struct VolumeUsageState {
    removing: bool,
    replacing: bool,
    active: usize,
}

pub(super) struct VolumeUsage {
    state: Mutex<VolumeUsageState>,
    changed: Condvar,
}

pub(crate) struct VolumeUseGuard {
    usage: Arc<VolumeUsage>,
}

impl VolumeUseGuard {
    fn new(usage: Arc<VolumeUsage>) -> Self {
        Self { usage }
    }
}

impl Drop for VolumeUseGuard {
    fn drop(&mut self) {
        let mut state = self.usage.state.lock();
        state.active = state.active.saturating_sub(1);
        if state.active == 0 {
            self.usage.changed.notify_all();
        }
    }
}

impl Default for VolumeUsage {
    fn default() -> Self {
        Self {
            state: Mutex::new(VolumeUsageState::default()),
            changed: Condvar::new(),
        }
    }
}

impl VolumeUsage {
    pub(super) fn is_removing(&self) -> bool {
        let state = self.state.lock();
        state.removing || state.replacing
    }

    pub(super) fn acquire(self: &Arc<Self>) -> Option<VolumeUseGuard> {
        let mut state = self.state.lock();
        if state.removing || state.replacing {
            return None;
        }
        state.active += 1;
        Some(VolumeUseGuard::new(self.clone()))
    }

    pub(super) fn begin_removal(&self) {
        self.state.lock().removing = true;
    }

    pub(super) fn begin_replacement(&self) {
        self.state.lock().replacing = true;
    }

    pub(super) fn replacement_is_idle(&self) -> bool {
        let state = self.state.lock();
        state.replacing && !state.removing && state.active == 0
    }

    pub(super) fn cancel_removal(&self) {
        self.state.lock().removing = false;
        self.changed.notify_all();
    }

    pub(super) fn wait_until_idle(&self, deadline: std::time::Instant) -> bool {
        let mut state = self.state.lock();
        while state.active > 0 {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return false;
            }
            self.changed.wait_for(&mut state, remaining);
        }
        true
    }
}
