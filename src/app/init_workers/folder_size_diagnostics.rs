//! Numeric IPC timings, with no path data. Request IDs correlate starts and
//! finishes even if the OS or service delays a query for several seconds.
use crate::infrastructure::diagnostic_logger::{
    self as diag, field_duration_ms, field_label, field_u64,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub(super) struct Query {
    started: Instant,
    id: u64,
    attempts: u64,
    outcome: &'static str,
    session: Option<u64>,
}

impl Query {
    pub(super) fn new() -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        diag::diag_info(
            "folder_size_query",
            "started",
            &[field_u64("request_id", id)],
        );
        Self {
            started: Instant::now(),
            id,
            attempts: 0,
            outcome: "failed",
            session: diag::capture_session(),
        }
    }
    pub(super) fn attempt(&mut self) {
        self.attempts += 1;
    }
    pub(super) fn outcome(&mut self, outcome: &'static str) {
        self.outcome = outcome;
    }
}

impl Drop for Query {
    fn drop(&mut self) {
        if self.session.is_none() || self.session != diag::capture_session() {
            return;
        }
        diag::diag_info(
            "folder_size_query",
            "finished",
            &[
                field_u64("request_id", self.id),
                field_u64("attempts", self.attempts),
                field_duration_ms("duration", self.started.elapsed()),
                field_label("outcome", self.outcome),
            ],
        );
    }
}
