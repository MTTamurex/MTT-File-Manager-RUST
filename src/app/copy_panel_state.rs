use std::collections::{HashMap, VecDeque};
use std::time::Duration;

#[derive(Default)]
pub struct CopyPanelState {
    pub metrics: HashMap<u64, TransferMetrics>,
    pub minimized: bool,
}

impl CopyPanelState {
    pub fn minimize(&mut self) {
        self.minimized = true;
    }
    pub fn restore(&mut self) {
        self.minimized = false;
    }
}

#[derive(Default)]
pub struct TransferMetrics {
    last: Option<(u64, Duration)>,
    sampling: bool,
    pub speed: f64,
    pub history: VecDeque<f32>,
}

impl TransferMetrics {
    /// The clock already excludes pause intervals. Reset the baseline when
    /// leaving pause/preflight so waiting time and draining I/O cannot spike ETA.
    pub fn sample(&mut self, bytes: u64, active_time: Duration, sampling: bool) {
        if !sampling || !self.sampling {
            self.last = Some((bytes, active_time));
            self.speed = 0.0;
            self.sampling = sampling;
            return;
        }
        let Some((previous, at)) = self.last else {
            return;
        };
        let elapsed = active_time.saturating_sub(at).as_secs_f64();
        if elapsed < 0.25 {
            return;
        }
        self.speed = bytes.saturating_sub(previous) as f64 / elapsed;
        self.last = Some((bytes, active_time));
        if self.history.len() == 120 {
            self.history.pop_front();
        }
        self.history.push_back((self.speed / 1_048_576.0) as f32);
    }

    pub fn remaining(&self, copied: u64, total: u64) -> Option<Duration> {
        if self.speed <= 0.0 {
            return None;
        }
        Duration::try_from_secs_f64(total.saturating_sub(copied) as f64 / self.speed).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimizing_and_restoring_preserves_existing_metrics() {
        let mut panel = CopyPanelState::default();
        let metrics = panel.metrics.entry(1).or_default();
        metrics.sample(0, Duration::ZERO, true);
        metrics.sample(100, Duration::from_secs(1), true);
        panel.minimize();
        assert!(panel.minimized);
        panel.restore();
        assert!(!panel.minimized);
        assert_eq!(panel.metrics[&1].history.len(), 1);
        assert_eq!(panel.metrics[&1].speed, 100.0);
    }

    #[test]
    fn pause_and_resume_reset_speed_without_losing_history() {
        let mut metrics = TransferMetrics::default();
        metrics.sample(0, Duration::ZERO, true);
        metrics.sample(100, Duration::from_secs(1), true);
        assert_eq!(metrics.speed, 100.0);
        assert_eq!(metrics.remaining(100, 300), Some(Duration::from_secs(2)));
        metrics.sample(110, Duration::from_secs(1), false);
        assert_eq!(metrics.speed, 0.0);
        assert_eq!(metrics.remaining(110, 300), None);
        metrics.sample(110, Duration::from_secs(1), true);
        metrics.sample(210, Duration::from_secs(2), true);
        assert_eq!(metrics.speed, 100.0);
        assert_eq!(metrics.history.len(), 2);
    }

    #[test]
    fn rollback_never_produces_negative_speed_and_history_is_bounded() {
        let mut metrics = TransferMetrics::default();
        metrics.sample(100, Duration::ZERO, true);
        metrics.sample(0, Duration::from_secs(1), true);
        assert_eq!(metrics.speed, 0.0);
        for tick in 2..200 {
            metrics.sample(tick, Duration::from_secs(tick), true);
        }
        assert_eq!(metrics.history.len(), 120);
    }
}
