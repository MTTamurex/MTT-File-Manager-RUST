//! Timings for UI logic outside the painted frame. No work when disabled.
use super::{capture_session, field_duration_ms, field_label, frame_trace, writer, LOGGER};
use std::time::{Duration, Instant};

pub(crate) struct SlowScope {
    session: Option<u64>,
    started: Option<Instant>,
    name: &'static str,
}

impl SlowScope {
    pub(crate) fn new(name: &'static str) -> Self {
        let session = capture_session();
        Self {
            session,
            started: session.map(|_| Instant::now()),
            name,
        }
    }
}

impl Drop for SlowScope {
    fn drop(&mut self) {
        let (Some(session), Some(started)) = (self.session, self.started) else {
            return;
        };
        let elapsed = started.elapsed();
        if elapsed < Duration::from_millis(50) || capture_session() != Some(session) {
            return;
        }
        frame_trace::request_dump(self.name);
        if let Some(logger) = LOGGER.get() {
            logger.writer.critical_events(
                session,
                vec![writer::Event::new(
                    log::Level::Warn,
                    "frame_trace",
                    "slow_scope",
                    vec![
                        field_label("scope", self.name),
                        field_duration_ms("duration", elapsed),
                    ],
                )],
            );
        }
    }
}
