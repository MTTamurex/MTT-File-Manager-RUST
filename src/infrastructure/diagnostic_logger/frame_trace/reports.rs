//! Keep one rejected stall report intact while output is congested. Retrying
//! is rate-limited; subsequent frames use a separate bounded history.
use super::{report, writer, Summary, HISTORY, SLOW_FRAME};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const RETRY_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Default)]
pub(super) struct Reports {
    history: VecDeque<Summary>,
    pending: Option<Vec<writer::Event>>,
    retry_at: Option<Instant>,
    reason: Option<&'static str>,
}

impl Reports {
    pub(super) fn record(
        &mut self,
        summary: Summary,
        reason: Option<&'static str>,
        now: Instant,
    ) -> Option<Vec<writer::Event>> {
        if self.pending.is_some() {
            if reason.is_some() || summary.duration >= SLOW_FRAME {
                self.reason = reason.or(Some("slow_frame"));
            }
            if self.history.len() == HISTORY {
                self.history.pop_front();
            }
            self.history.push_back(summary);
            if self.retry_at.is_some_and(|at| now < at) {
                return None;
            }
            return self.pending.take();
        }
        let events = report(&mut self.history, summary, reason.or(self.reason.take()));
        (!events.is_empty()).then_some(events)
    }

    pub(super) fn reject(&mut self, events: Vec<writer::Event>, now: Instant) {
        self.pending = Some(events);
        self.retry_at = Some(now + RETRY_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn summary(id: u64, duration: u64) -> Summary {
        Summary {
            id,
            captured_at_epoch_ms: 0,
            duration: Duration::from_millis(duration),
            gap: Duration::ZERO,
            stages: vec![],
            context: vec![],
        }
    }

    #[test]
    fn rejected_stall_survives_more_than_a_history_of_new_frames_without_busy_retries() {
        let mut reports = Reports::default();
        let now = Instant::now();
        let events = reports.record(summary(1, 2000), None, now).unwrap();
        let original_count = events.len();
        reports.reject(events, now);
        for id in 2..100 {
            assert!(reports.record(summary(id, 1), None, now).is_none());
        }
        assert_eq!(reports.history.len(), HISTORY);
        let retried = reports
            .record(summary(100, 1), None, now + RETRY_INTERVAL)
            .unwrap();
        assert_eq!(retried.len(), original_count);
        assert!(reports.pending.is_none());
    }

    #[test]
    fn another_stall_while_congested_requests_a_later_history_dump() {
        let mut reports = Reports::default();
        let now = Instant::now();
        let events = reports.record(summary(1, 2000), None, now).unwrap();
        reports.reject(events, now);
        assert!(reports.record(summary(2, 3000), None, now).is_none());
        assert!(reports
            .record(summary(3, 1), None, now + RETRY_INTERVAL)
            .is_some());
        assert!(reports
            .record(summary(4, 1), None, now + RETRY_INTERVAL)
            .is_some());
    }
}
