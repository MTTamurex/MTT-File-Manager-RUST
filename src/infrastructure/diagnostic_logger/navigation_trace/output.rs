//! Navigation and continuous frame capture share the same bounded writer.
use crate::infrastructure::diagnostic_logger::{capture_session, writer, DiagnosticField, LOGGER};

pub(super) struct Event(pub &'static str, pub Vec<DiagnosticField>);

pub(super) fn emit(session: u64, events: Vec<Event>) {
    if capture_session() != Some(session) {
        return;
    }
    if let Some(logger) = LOGGER.get() {
        logger.writer.events(
            session,
            events
                .into_iter()
                .map(|Event(code, fields)| {
                    writer::Event::new(log::Level::Info, "navigation_trace", code, fields)
                })
                .collect(),
        );
    }
}
