//! Always-on (during diagnosis) bounded in-memory flight recorder.
//! Gaps between frames can be intentional idle; only execution time triggers a dump.
mod reports;
use super::{
    capture_session, field_bool, field_duration_ms, field_label, field_u64, writer,
    DiagnosticField, LOGGER,
};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const HISTORY: usize = 32;
const MAX_STAGES: usize = 128;
const RESERVED_CHECKPOINTS: usize = 16;
const SLOW_FRAME: Duration = Duration::from_millis(50);
static LAST_TRIM_EPOCH_MS: AtomicU64 = AtomicU64::new(0);
static TRIM_COUNT: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
struct Recorder {
    session: Option<u64>,
    next_id: u64,
    active: Option<(u64, Vec<(&'static str, Duration)>)>,
    reports: reports::Reports,
    context: Vec<DiagnosticField>,
    dump_reason: Option<&'static str>,
}

struct Summary {
    id: u64,
    captured_at_epoch_ms: u64,
    duration: Duration,
    gap: Duration,
    stages: Vec<(&'static str, Duration)>,
    context: Vec<DiagnosticField>,
}

thread_local! { static RECORDER: RefCell<Recorder> = RefCell::new(Recorder::default()); }

pub(crate) struct Frame {
    session: u64,
    id: u64,
    started: Instant,
    gap: Duration,
    captured_at_epoch_ms: u64,
}

fn epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

pub(crate) fn memory_trim_completed() {
    LAST_TRIM_EPOCH_MS.store(epoch_ms(), Ordering::Relaxed);
    TRIM_COUNT.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn context(fields: Vec<DiagnosticField>) {
    RECORDER.with(|r| {
        let mut r = r.borrow_mut();
        if r.active.is_some() {
            r.context = fields;
        }
    });
}

pub(crate) fn request_dump(reason: &'static str) {
    let Some(session) = capture_session() else {
        return;
    };
    RECORDER.with(|r| {
        let mut r = r.borrow_mut();
        if r.session != Some(session) {
            *r = Recorder {
                session: Some(session),
                ..Default::default()
            };
        }
        r.dump_reason = Some(reason);
    });
}

impl Frame {
    pub(crate) fn new(previous_exit: Option<Instant>) -> Option<Self> {
        let session = capture_session();
        RECORDER.with(|r| {
            let mut r = r.borrow_mut();
            if r.session != session {
                *r = Recorder {
                    session,
                    ..Default::default()
                };
            }
            let session = session?;
            r.next_id = r.next_id.wrapping_add(1);
            let id = r.next_id;
            r.active = Some((id, Vec::new()));
            r.context.clear();
            let started = Instant::now();
            Some(Self {
                session,
                id,
                started,
                gap: previous_exit
                    .map(|t| started.saturating_duration_since(t))
                    .unwrap_or_default(),
                captured_at_epoch_ms: epoch_ms(),
            })
        })
    }

    pub(crate) fn checkpoint(&self, name: &'static str, elapsed: Duration) {
        append(self.id, name, elapsed, MAX_STAGES);
    }

    pub(crate) fn finish(self, loading: bool, items: usize, generation: usize) {
        let duration = self.started.elapsed();
        let events = RECORDER.with(|r| {
            let mut r = r.borrow_mut();
            if r.session != Some(self.session)
                || r.active.as_ref().is_none_or(|(id, _)| *id != self.id)
                || capture_session() != Some(self.session)
            {
                return None;
            }
            let (id, stages) = r.active.take()?;
            let mut context = std::mem::take(&mut r.context);
            context.extend([
                field_bool("loading", loading),
                field_u64("items", items as u64),
                field_u64("generation", generation as u64),
                field_u64("trim_count", TRIM_COUNT.load(Ordering::Relaxed)),
                field_u64(
                    "last_trim_epoch_ms",
                    LAST_TRIM_EPOCH_MS.load(Ordering::Relaxed),
                ),
            ]);
            let summary = Summary {
                id,
                captured_at_epoch_ms: self.captured_at_epoch_ms,
                duration,
                gap: self.gap,
                stages,
                context,
            };
            let reason = r.dump_reason.take();
            r.reports.record(summary, reason, Instant::now())
        });
        if let (Some(logger), Some(events)) = (LOGGER.get(), events) {
            if let Err(events) = logger.writer.try_critical_events(self.session, events) {
                RECORDER.with(|r| {
                    let mut r = r.borrow_mut();
                    if r.session == Some(self.session) {
                        r.reports.reject(events, Instant::now());
                    }
                });
            }
        }
    }
}

fn append(id: u64, name: &'static str, elapsed: Duration, limit: usize) {
    RECORDER.with(|r| {
        if let Some((active, stages)) = &mut r.borrow_mut().active {
            if *active == id && stages.len() < limit {
                stages.push((name, elapsed));
            }
        }
    });
}

/// Nested stages are inclusive; they must not be added to their parent timings.
pub(crate) struct Stage {
    id: Option<u64>,
    name: &'static str,
    started: Option<Instant>,
}

impl Stage {
    pub(crate) fn new(name: &'static str) -> Self {
        let id = RECORDER.with(|r| r.borrow().active.as_ref().map(|(id, _)| *id));
        Self {
            id,
            name,
            started: id.map(|_| Instant::now()),
        }
    }

    pub(crate) fn lap(&mut self, next: &'static str) {
        if let (Some(id), Some(started)) = (self.id, self.started) {
            append(
                id,
                self.name,
                started.elapsed(),
                MAX_STAGES - RESERVED_CHECKPOINTS,
            );
            self.started = (next != "finished").then(Instant::now);
        }
        self.name = next;
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        self.lap("finished");
    }
}

fn report(
    history: &mut VecDeque<Summary>,
    summary: Summary,
    reason: Option<&'static str>,
) -> Vec<writer::Event> {
    let slow = summary.duration >= SLOW_FRAME || reason.is_some();
    if history.len() == HISTORY {
        history.pop_front();
    }
    history.push_back(summary);
    if !slow {
        return Vec::new();
    }
    let mut events = Vec::new();
    for frame in history.drain(..) {
        let mut fields = vec![
            field_u64("frame", frame.id),
            field_u64("frame_started_epoch_ms", frame.captured_at_epoch_ms),
            field_duration_ms("duration", frame.duration),
            field_duration_ms("ui_gap", frame.gap),
            field_bool("slow", frame.duration >= SLOW_FRAME),
            field_label("trigger", reason.unwrap_or("slow_frame")),
        ];
        fields.extend(frame.context);
        events.push(writer::Event::new(
            log::Level::Info,
            "frame_trace",
            "frame",
            fields,
        ));
        for (stage, duration) in frame.stages {
            events.push(writer::Event::new(
                log::Level::Info,
                "frame_trace",
                "stage",
                vec![
                    field_u64("frame", frame.id),
                    field_label("stage", stage),
                    field_duration_ms("duration", duration),
                ],
            ));
        }
    }
    events
}

impl Drop for Frame {
    fn drop(&mut self) {
        RECORDER.with(|r| {
            let mut r = r.borrow_mut();
            if r.session == Some(self.session)
                && r.active.as_ref().is_some_and(|(id, _)| *id == self.id)
            {
                r.active = None;
                r.context.clear();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn summary(id: u64, ms: u64) -> Summary {
        Summary {
            id,
            captured_at_epoch_ms: 0,
            duration: Duration::from_millis(ms),
            gap: Duration::from_secs(60),
            stages: vec![("test", Duration::from_millis(ms))],
            context: Vec::new(),
        }
    }
    #[test]
    fn idle_gaps_do_not_trigger_and_history_is_bounded() {
        let mut history = VecDeque::new();
        for id in 0..100 {
            assert!(report(&mut history, summary(id, 1), None).is_empty());
        }
        assert_eq!(history.len(), HISTORY);
        let events = report(&mut history, summary(100, 2000), None);
        assert_eq!(events.len(), HISTORY * 2);
        assert!(history.is_empty());
    }
    #[test]
    fn records_slow_frame_without_sidebar_navigation() {
        let events = report(&mut VecDeque::new(), summary(1, 3500), None);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn confirmed_execution_gap_preserves_history_even_with_a_fast_ui_frame() {
        let mut history = VecDeque::new();
        assert!(report(&mut history, summary(1, 1), None).is_empty());
        let events = report(&mut history, summary(2, 1), Some("paint_interval"));
        assert_eq!(events.len(), 4);
        assert!(history.is_empty());
    }

    #[test]
    fn nested_stages_are_bounded_without_an_active_navigation_trace() {
        RECORDER.with(|r| r.borrow_mut().active = Some((1, Vec::new())));
        for _ in 0..MAX_STAGES + 5 {
            drop(Stage::new("background_messages"));
        }
        RECORDER.with(|r| {
            let mut r = r.borrow_mut();
            let (_, stages) = r.active.take().unwrap();
            assert_eq!(stages.len(), MAX_STAGES - RESERVED_CHECKPOINTS);
            assert!(stages
                .iter()
                .all(|(name, _)| *name == "background_messages"));
        });
    }

    #[test]
    fn abandoning_a_frame_does_not_attach_later_stages_to_it() {
        RECORDER.with(|r| {
            let mut r = r.borrow_mut();
            r.session = Some(1);
            r.active = Some((1, Vec::new()));
        });
        drop(Frame {
            session: 1,
            id: 1,
            started: Instant::now(),
            gap: Duration::ZERO,
            captured_at_epoch_ms: 0,
        });
        assert!(Stage::new("after_return").started.is_none());
    }

    #[test]
    fn nested_stage_overflow_does_not_hide_top_level_checkpoints() {
        RECORDER.with(|r| r.borrow_mut().active = Some((1, Vec::new())));
        for _ in 0..MAX_STAGES + 5 {
            drop(Stage::new("nested"));
        }
        append(1, "frame_panels", Duration::from_secs(2), MAX_STAGES);
        RECORDER.with(|r| {
            let (_, stages) = r.borrow_mut().active.take().unwrap();
            assert_eq!(stages.last().unwrap().0, "frame_panels");
        });
    }
}
