//! Schedules the single wakeup that lets the visible-idle working-set trim run.
//!
//! The idle trim is evaluated inside a frame, but idle frames are only
//! requested every ~30 s. Without a dedicated deadline the trim would wait for
//! the next unrelated wakeup instead of running shortly after the user stops
//! interacting. At most one extra frame is requested per idle period.
//!
//! Losing focus is a strong signal that the user moved to another app, so an
//! unfocused window becomes due shortly after focus loss instead of waiting
//! for the full no-interaction delay.

use std::time::{Duration, Instant};

use super::{
    process_working_set_trim_disabled, ImageViewerApp, LOW_RAM_GPU_IDLE_WS_TRIM_AFTER,
    WORKING_SET_TRIM_MIN_INTERVAL,
};

/// Matches the periodic throttle in `run_memory_maintenance_impl`.
const MEMORY_MAINTENANCE_INTERVAL: Duration = Duration::from_secs(2);
/// Settle time after focus loss before an unfocused window is trimmed. Short
/// enough to feel immediate, long enough to skip quick Alt+Tab round trips.
const UNFOCUSED_IDLE_TRIM_AFTER: Duration = Duration::from_secs(2);

/// When the idle trim becomes due: after the no-interaction delay, or sooner
/// once the window has been unfocused for a short settle time.
fn idle_trim_due_at(last_user_activity: Instant, focus_lost_at: Option<Instant>) -> Instant {
    let after_activity = last_user_activity + LOW_RAM_GPU_IDLE_WS_TRIM_AFTER;
    focus_lost_at.map_or(after_activity, |lost| {
        after_activity.min(lost + UNFOCUSED_IDLE_TRIM_AFTER)
    })
}

/// Returns how long to wait before the frame that evaluates the idle trim, or
/// `None` once this idle period has already been evaluated.
fn idle_trim_wakeup_in(
    now: Instant,
    last_user_activity: Instant,
    due_at: Instant,
    last_restore_time: Instant,
    evaluated_for: Option<Instant>,
    last_maintenance: Instant,
) -> Option<Duration> {
    let anchor = last_user_activity.max(last_restore_time);
    if evaluated_for == Some(anchor) {
        return None;
    }
    let remaining =
        |start: Instant, interval: Duration| (start + interval).saturating_duration_since(now);
    Some(
        due_at
            .saturating_duration_since(now)
            .max(remaining(last_restore_time, WORKING_SET_TRIM_MIN_INTERVAL))
            .max(remaining(last_maintenance, MEMORY_MAINTENANCE_INTERVAL)),
    )
}

impl ImageViewerApp {
    fn idle_trim_anchor(&self) -> Instant {
        self.last_user_activity.max(self.last_restore_time)
    }

    /// Whether the no-interaction (or unfocused) delay of the idle trim passed.
    pub(super) fn idle_trim_delay_elapsed(&self) -> bool {
        Instant::now() >= idle_trim_due_at(self.last_user_activity, self.focus_lost_at)
    }

    /// Deadline for the frame that evaluates the visible-idle trim.
    pub(crate) fn idle_trim_wakeup_in(&self) -> Option<Duration> {
        if !self.uses_aggressive_gpu_memory_policy() || self.background_memory_trim_active {
            return None;
        }
        let wakeup = idle_trim_wakeup_in(
            Instant::now(),
            self.last_user_activity,
            idle_trim_due_at(self.last_user_activity, self.focus_lost_at),
            self.last_restore_time,
            self.idle_trim_evaluated_for,
            self.last_memory_maintenance,
        )?;
        // Hidden viewports skip `run_memory_maintenance`, so the trim could
        // never be evaluated there and the deadline would keep firing.
        let visible = self
            .ui_ctx
            .input(|input| input.viewport().visible().unwrap_or(true));
        (visible
            && !crate::infrastructure::windows::window_subclass::is_backgrounded()
            && !process_working_set_trim_disabled())
        .then_some(wakeup)
    }

    /// Records that the current idle period reached its trim evaluation, so no
    /// further dedicated wakeups are requested until new activity.
    pub(super) fn mark_idle_trim_evaluated(&mut self) {
        self.idle_trim_evaluated_for = Some(self.idle_trim_anchor());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_the_idle_delay_after_activity() {
        let now = Instant::now();
        let activity = now;
        let restore = now - Duration::from_secs(60);
        let maintenance = now - Duration::from_secs(60);
        assert_eq!(
            idle_trim_wakeup_in(
                now,
                activity,
                idle_trim_due_at(activity, None),
                restore,
                None,
                maintenance
            ),
            Some(LOW_RAM_GPU_IDLE_WS_TRIM_AFTER)
        );
        let later = now + Duration::from_secs(3);
        assert_eq!(
            idle_trim_wakeup_in(
                later,
                activity,
                idle_trim_due_at(activity, None),
                restore,
                None,
                maintenance
            ),
            Some(LOW_RAM_GPU_IDLE_WS_TRIM_AFTER - Duration::from_secs(3))
        );
    }

    #[test]
    fn respects_the_post_restore_interval() {
        let now = Instant::now();
        let activity = now - Duration::from_secs(60);
        let maintenance = now - Duration::from_secs(60);
        assert_eq!(
            idle_trim_wakeup_in(
                now,
                activity,
                idle_trim_due_at(activity, None),
                now,
                None,
                maintenance
            ),
            Some(WORKING_SET_TRIM_MIN_INTERVAL)
        );
    }

    #[test]
    fn never_requests_a_wakeup_inside_the_maintenance_throttle() {
        let now = Instant::now();
        let activity = now - Duration::from_secs(60);
        let restore = now - Duration::from_secs(60);
        let maintenance = now - Duration::from_millis(500);
        assert_eq!(
            idle_trim_wakeup_in(
                now,
                activity,
                idle_trim_due_at(activity, None),
                restore,
                None,
                maintenance
            ),
            Some(Duration::from_millis(1500))
        );
    }

    #[test]
    fn stops_after_evaluation_and_rearms_on_new_activity() {
        let now = Instant::now();
        let activity = now - Duration::from_secs(20);
        let restore = now - Duration::from_secs(60);
        let maintenance = now - Duration::from_secs(60);
        assert_eq!(
            idle_trim_wakeup_in(
                now,
                activity,
                idle_trim_due_at(activity, None),
                restore,
                Some(activity),
                maintenance
            ),
            None
        );
        assert_eq!(
            idle_trim_wakeup_in(
                now,
                now,
                idle_trim_due_at(now, None),
                restore,
                Some(activity),
                maintenance
            ),
            Some(LOW_RAM_GPU_IDLE_WS_TRIM_AFTER)
        );
    }

    #[test]
    fn unfocused_window_is_due_shortly_after_focus_loss() {
        let activity = Instant::now();
        let lost = activity + Duration::from_secs(1);
        assert_eq!(
            idle_trim_due_at(activity, Some(lost)),
            lost + UNFOCUSED_IDLE_TRIM_AFTER
        );
        // Focus lost long after the last interaction: the earlier deadline wins.
        let late_loss = activity + Duration::from_secs(30);
        assert_eq!(
            idle_trim_due_at(activity, Some(late_loss)),
            activity + LOW_RAM_GPU_IDLE_WS_TRIM_AFTER
        );
        assert_eq!(
            idle_trim_due_at(activity, None),
            activity + LOW_RAM_GPU_IDLE_WS_TRIM_AFTER
        );
    }
}
