use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use super::{
    current_process_memory_snapshot, process_working_set_trim_disabled,
    request_process_working_set_trim_series_with_completion, ImageViewerApp,
    LOW_RAM_GPU_IDLE_WS_TRIM_MIN_BYTES, WORKING_SET_TRIM_EPOCH,
};

const STARTUP_SETTLE_TIME: Duration = Duration::from_secs(1);
const STARTUP_TRIM_DELAYS: &[Duration] = &[Duration::ZERO];

#[derive(Default)]
pub(crate) struct StartupMemoryTrim {
    finished: bool,
    restore_time: Option<Instant>,
    settled_since: Option<Instant>,
    settled_epoch: u64,
    completion: Option<Receiver<bool>>,
}

impl From<Instant> for StartupMemoryTrim {
    fn from(restore_time: Instant) -> Self {
        Self {
            restore_time: Some(restore_time),
            ..Self::default()
        }
    }
}

impl StartupMemoryTrim {
    fn wait_remaining(
        &mut self,
        now: Instant,
        blocked: bool,
        last_activity: Instant,
        epoch: u64,
    ) -> Duration {
        if blocked || self.settled_epoch != epoch {
            self.settled_since = None;
        }
        self.settled_epoch = epoch;
        if blocked {
            return STARTUP_SETTLE_TIME;
        }
        let since = self.settled_since.get_or_insert(now);
        *since = (*since).max(last_activity);
        STARTUP_SETTLE_TIME.saturating_sub(now.duration_since(*since))
    }
}

impl ImageViewerApp {
    pub(super) fn run_startup_memory_trim(&mut self) {
        if self.startup_memory_trim.finished {
            return;
        }
        let initial_restore_time = self
            .startup_memory_trim
            .restore_time
            .get_or_insert(self.last_restore_time);
        // Never carry the startup exception across a minimize/restore cycle.
        if *initial_restore_time != self.last_restore_time
            || self.background_memory_trim_active
            || !self.uses_aggressive_gpu_memory_policy()
            || process_working_set_trim_disabled()
        {
            self.startup_memory_trim.finished = true;
            return;
        }

        if let Some(completion) = &self.startup_memory_trim.completion {
            match completion.try_recv() {
                Ok(true) | Err(TryRecvError::Disconnected) => {
                    self.startup_memory_trim.finished = true;
                    return;
                }
                Ok(false) => {
                    self.startup_memory_trim.completion = None;
                    self.startup_memory_trim.settled_since = None;
                }
                Err(TryRecvError::Empty) => {
                    self.ui_ctx
                        .request_repaint_after(Duration::from_millis(100));
                    return;
                }
            }
        }

        self.refresh_working_set_trim_blocker(false);
        // User activity is already accounted for by last_activity below; do not
        // add a second one-second grace period on top of the existing blocker.
        let loading_or_busy = self
            .working_set_trim_blocker_reason_with_activity_grace(false)
            .is_some();
        let wait = self.startup_memory_trim.wait_remaining(
            Instant::now(),
            loading_or_busy,
            self.last_user_activity,
            WORKING_SET_TRIM_EPOCH.load(Ordering::Acquire),
        );
        if !wait.is_zero() {
            self.ui_ctx.request_repaint_after(wait);
            return;
        }
        let Some(snapshot) = current_process_memory_snapshot() else {
            self.ui_ctx.request_repaint_after(STARTUP_SETTLE_TIME);
            return;
        };
        if snapshot.working_set_bytes < LOW_RAM_GPU_IDLE_WS_TRIM_MIN_BYTES {
            self.startup_memory_trim.finished = true;
            return;
        }

        let (sender, receiver) = mpsc::channel();
        if request_process_working_set_trim_series_with_completion(
            format!("startup settled backend={}", self.active_gpu_backend),
            STARTUP_TRIM_DELAYS,
            false,
            Some(sender),
        ) {
            self.startup_memory_trim.completion = Some(receiver);
        }
        self.ui_ctx.request_repaint_after(STARTUP_SETTLE_TIME);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_waits_one_second_after_loading_finishes() {
        let now = Instant::now();
        let mut trim = StartupMemoryTrim::default();
        assert_eq!(trim.wait_remaining(now, true, now, 0), STARTUP_SETTLE_TIME);
        let loaded = now + Duration::from_secs(2);
        assert_eq!(
            trim.wait_remaining(loaded, false, now, 0),
            STARTUP_SETTLE_TIME
        );
        assert_eq!(
            trim.wait_remaining(loaded + STARTUP_SETTLE_TIME, false, now, 0),
            Duration::ZERO
        );
    }

    #[test]
    fn interaction_restarts_the_settle_margin() {
        let now = Instant::now();
        let mut trim = StartupMemoryTrim::default();
        trim.wait_remaining(now, false, now, 0);
        let activity = now + Duration::from_millis(800);
        assert_eq!(
            trim.wait_remaining(activity, false, activity, 0),
            STARTUP_SETTLE_TIME
        );
        assert_eq!(
            trim.wait_remaining(activity + STARTUP_SETTLE_TIME, false, activity, 0),
            Duration::ZERO
        );
    }

    #[test]
    fn new_work_between_checks_restarts_the_settle_margin() {
        let now = Instant::now();
        let mut trim = StartupMemoryTrim::default();
        trim.wait_remaining(now, false, now, 0);
        assert_eq!(
            trim.wait_remaining(now + STARTUP_SETTLE_TIME, false, now, 1),
            STARTUP_SETTLE_TIME
        );
    }

    #[test]
    fn loading_resuming_during_the_margin_restarts_it() {
        let now = Instant::now();
        let mut trim = StartupMemoryTrim::default();
        trim.wait_remaining(now, false, now, 0);
        trim.wait_remaining(now + Duration::from_millis(500), true, now, 1);
        let loaded = now + STARTUP_SETTLE_TIME;
        assert_eq!(
            trim.wait_remaining(loaded, false, now, 1),
            STARTUP_SETTLE_TIME
        );
        assert_eq!(
            trim.wait_remaining(loaded + STARTUP_SETTLE_TIME, false, now, 1),
            Duration::ZERO
        );
    }
}
