use std::time::{Duration, Instant};

const INITIAL_DELAY: Duration = Duration::from_secs(1);
const MAX_DELAY: Duration = Duration::from_secs(60);

pub(super) struct WatcherRetryState {
    failures: u32,
    retry_at: Instant,
}

impl WatcherRetryState {
    pub(super) fn after_failure(previous: Option<&Self>, now: Instant) -> Self {
        let failures = previous
            .map(|state| state.failures.saturating_add(1))
            .unwrap_or(1);
        Self {
            failures,
            retry_at: now + retry_delay(failures),
        }
    }

    pub(super) fn failures(&self) -> u32 {
        self.failures
    }

    pub(super) fn is_ready(&self, now: Instant) -> bool {
        now >= self.retry_at
    }

    pub(super) fn retry_in(&self, now: Instant) -> Duration {
        self.retry_at.saturating_duration_since(now)
    }
}

fn retry_delay(failures: u32) -> Duration {
    let exponent = failures.saturating_sub(1).min(6);
    INITIAL_DELAY
        .saturating_mul(1u32 << exponent)
        .min(MAX_DELAY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_delay_grows_and_is_capped() {
        assert_eq!(retry_delay(1), Duration::from_secs(1));
        assert_eq!(retry_delay(2), Duration::from_secs(2));
        assert_eq!(retry_delay(6), Duration::from_secs(32));
        assert_eq!(retry_delay(7), Duration::from_secs(60));
        assert_eq!(retry_delay(u32::MAX), Duration::from_secs(60));
    }

    #[test]
    fn retry_state_blocks_until_its_own_deadline() {
        let now = Instant::now();
        let first = WatcherRetryState::after_failure(None, now);
        assert!(!first.is_ready(now));
        assert!(first.is_ready(now + Duration::from_secs(1)));

        let second = WatcherRetryState::after_failure(Some(&first), now);
        assert_eq!(second.failures(), 2);
        assert!(!second.is_ready(now + Duration::from_secs(1)));
        assert!(second.is_ready(now + Duration::from_secs(2)));
    }
}
