//! Per-tracker hardware cadence, invalidated with the drive generation.
use std::time::{Duration, Instant};

const INTERVAL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Default)]
pub(crate) struct HardwareSchedule {
    generation: u64,
    attempts: rustc_hash::FxHashMap<String, Instant>,
}

impl HardwareSchedule {
    pub(crate) fn claim(&mut self, path: &str, generation: u64, now: Instant) -> bool {
        if generation != self.generation {
            return false;
        }
        // Only actual drive roots are queried; this also bounds the map to 26.
        let Some(key) = crate::app::drive_state::normalize_drive_root_key(path) else {
            return false;
        };
        if self
            .attempts
            .get(&key)
            .is_some_and(|last| now.saturating_duration_since(*last) < INTERVAL)
        {
            return false;
        }
        self.attempts.insert(key, now);
        true
    }

    pub(super) fn invalidate(&mut self, generation: u64) {
        self.generation = generation;
        self.attempts.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cadence_is_case_insensitive_and_independent_per_drive() {
        let mut schedule = HardwareSchedule::default();
        let now = Instant::now();
        assert!(schedule.claim("Q:", 0, now));
        assert!(!schedule.claim(r"q:\", 0, now + INTERVAL - Duration::from_secs(1)));
        assert!(schedule.claim(r"q:\", 0, now + INTERVAL));
        assert!(schedule.claim("R:", 0, now));
    }

    #[test]
    fn invalidation_allows_a_replacement_drive_and_rejects_old_workers() {
        let mut schedule = HardwareSchedule::default();
        let now = Instant::now();
        assert!(schedule.claim("Q:", 0, now));
        schedule.invalidate(1);
        assert!(!schedule.claim("Q:", 0, now));
        assert!(schedule.claim("Q:", 1, now));
    }
}
