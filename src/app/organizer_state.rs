use crate::domain::organizer_rule::OrganizerRule;
use crate::infrastructure::organizer::OrganizerManager;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

const NOTIFICATION_IDLE_DELAY: Duration = Duration::from_millis(1500);

#[derive(Default)]
pub struct OrganizerNotificationBatch {
    moved: usize,
    skipped: usize,
    failed: usize,
    last_event_at: Option<Instant>,
}

pub struct OrganizerNotificationSummary {
    pub moved: usize,
    pub skipped: usize,
    pub failed: usize,
}

impl OrganizerNotificationBatch {
    pub fn record_moved(&mut self) {
        self.record_at(Instant::now(), |batch| batch.moved += 1);
    }

    pub fn record_skipped(&mut self) {
        self.record_at(Instant::now(), |batch| batch.skipped += 1);
    }

    pub fn record_failed(&mut self) {
        self.record_at(Instant::now(), |batch| batch.failed += 1);
    }

    pub fn take_if_idle(&mut self, now: Instant) -> Option<OrganizerNotificationSummary> {
        let last_event_at = self.last_event_at?;
        if now.duration_since(last_event_at) < NOTIFICATION_IDLE_DELAY {
            return None;
        }

        let summary = OrganizerNotificationSummary {
            moved: self.moved,
            skipped: self.skipped,
            failed: self.failed,
        };
        *self = Self::default();
        Some(summary)
    }

    fn record_at(&mut self, now: Instant, update: impl FnOnce(&mut Self)) {
        update(self);
        self.last_event_at = Some(now);
    }
}

pub struct OrganizerState {
    pub manager: OrganizerManager,
    pub rules: Vec<OrganizerRule>,
    pub source_input: String,
    pub destination_input: String,
    pub extensions_input: String,
    pub editing_rule_id: Option<i64>,
    pub notification_batch: OrganizerNotificationBatch,
}

impl OrganizerState {
    pub(crate) fn new(
        file_operation_sender: Sender<crate::workers::file_operation_worker::FileOperationRequest>,
        rules: Vec<OrganizerRule>,
        ui_ctx: eframe::egui::Context,
    ) -> Self {
        Self {
            manager: OrganizerManager::start(file_operation_sender, rules.clone(), ui_ctx),
            rules,
            source_input: String::new(),
            destination_input: String::new(),
            extensions_input: String::new(),
            editing_rule_id: None,
            notification_batch: OrganizerNotificationBatch::default(),
        }
    }

    pub fn reset_form(&mut self) {
        self.source_input.clear();
        self.destination_input.clear();
        self.extensions_input.clear();
        self.editing_rule_id = None;
    }

    pub fn replace_rules(&mut self, rules: Vec<OrganizerRule>) {
        self.manager.set_rules(rules.clone());
        self.rules = rules;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_batch_emits_one_summary_after_the_idle_delay() {
        let now = Instant::now();
        let mut batch = OrganizerNotificationBatch::default();
        batch.record_at(now, |batch| batch.moved += 2);
        batch.record_at(now + Duration::from_millis(500), |batch| batch.skipped += 1);

        assert!(batch
            .take_if_idle(now + Duration::from_millis(1500))
            .is_none());

        let summary = batch
            .take_if_idle(now + Duration::from_millis(2000))
            .expect("summary after idle delay");
        assert_eq!(summary.moved, 2);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.failed, 0);
        assert!(batch
            .take_if_idle(now + Duration::from_millis(4000))
            .is_none());
    }
}
