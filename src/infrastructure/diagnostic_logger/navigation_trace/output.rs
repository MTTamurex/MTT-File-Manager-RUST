//! Best-effort bounded writer: capture must never wait for diagnostic disk I/O.
use crate::infrastructure::diagnostic_logger::{
    capture_session, field_u64, DiagnosticField, DiagnosticLogger, LOGGER,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, OnceLock};

pub(super) struct Event(pub &'static str, pub Vec<DiagnosticField>);
struct Batch {
    session: u64,
    events: Vec<Event>,
}

static WRITER: OnceLock<Option<mpsc::SyncSender<Batch>>> = OnceLock::new();
static DROPPED: AtomicU64 = AtomicU64::new(0);

pub(super) fn emit(session: u64, events: Vec<Event>) {
    if capture_session() != Some(session) {
        return;
    }
    let Some(writer) = WRITER.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<Batch>(32);
        std::thread::Builder::new()
            .name("navigation-diagnostics".into())
            .spawn(move || {
                while let Ok(batch) = rx.recv() {
                    let Some(logger) = LOGGER.get() else { continue };
                    write_batch(logger, batch);
                }
            })
            .ok()
            .map(|_| tx)
    }) else {
        return;
    };
    let dropped = DROPPED.swap(0, Ordering::Relaxed);
    let mut events = events;
    if dropped > 0 {
        events.push(Event(
            "capture_overflow",
            vec![field_u64("dropped_batches", dropped)],
        ));
    }
    if !enqueue(writer, Batch { session, events }) {
        DROPPED.fetch_add(dropped.saturating_add(1), Ordering::Relaxed);
    }
}

fn enqueue(writer: &mpsc::SyncSender<Batch>, batch: Batch) -> bool {
    writer.try_send(batch).is_ok()
}

fn write_batch(logger: &DiagnosticLogger, batch: Batch) {
    let mut state = logger.state.lock();
    // Check under the file lock: never write old captures into a new session.
    if logger.capture_session() != Some(batch.session) {
        return;
    }
    if let Some(file) = state.file.as_mut() {
        for Event(code, fields) in batch.events {
            let _ = file.write_event(log::Level::Info, "navigation_trace", code, &fields);
        }
        let _ = file.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::diagnostic_logger::storage::DiagnosticFile;

    #[test]
    fn full_queue_drops_batch_without_waiting_for_consumer() {
        let (tx, _rx) = mpsc::sync_channel(1);
        assert!(enqueue(
            &tx,
            Batch {
                session: 1,
                events: Vec::new()
            }
        ));
        assert!(!enqueue(
            &tx,
            Batch {
                session: 1,
                events: Vec::new()
            }
        ));
    }

    #[test]
    fn delayed_batch_cannot_cross_diagnostic_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diagnostic.log");
        let logger = DiagnosticLogger::new(
            env_logger::Builder::new().build(),
            env_logger::Builder::new().build(),
        );
        logger.state.lock().file =
            Some(DiagnosticFile::open(path.clone(), std::time::SystemTime::now()).unwrap());
        logger.capture_session.store(3, Ordering::Release);
        write_batch(
            &logger,
            Batch {
                session: 1,
                events: vec![Event("old_capture", Vec::new())],
            },
        );
        write_batch(
            &logger,
            Batch {
                session: 3,
                events: vec![Event("current_capture", Vec::new())],
            },
        );
        let contents = std::fs::read_to_string(path).unwrap();
        assert!(!contents.contains("event=old_capture"));
        assert!(contents.contains("event=current_capture"));
    }
}
