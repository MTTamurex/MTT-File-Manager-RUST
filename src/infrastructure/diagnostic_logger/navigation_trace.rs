//! Bounded, privacy-safe timings for sidebar drive navigation. No filesystem probes.
//! UI timings are buffered until frame end so diagnostic writes are not timed as stages.

mod output;

use super::{capture_session, field_bool, field_duration_ms, field_label, field_u64};
use output::{emit, Event};
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const TRACE_WINDOW: Duration = Duration::from_secs(15);
const SLOW_FRAME: Duration = Duration::from_millis(50);
const MAX_STAGES: usize = 128;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static TRACE: RefCell<Option<Trace>> = const { RefCell::new(None) };
}

struct Trace {
    id: u64,
    session: u64,
    generation: Option<usize>,
    started: Instant,
    frames: u64,
    last_loading: bool,
    stages: Vec<(&'static str, Duration)>,
}

/// Called only for the drive row, not pinned folders or network shortcuts.
pub(crate) fn begin_sidebar_drive() {
    let Some(session) = capture_session() else {
        TRACE.with(|trace| *trace.borrow_mut() = None);
        return;
    };
    TRACE.with(|trace| {
        *trace.borrow_mut() = Some(Trace {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            session,
            generation: None,
            started: Instant::now(),
            frames: 0,
            last_loading: false,
            stages: Vec::new(),
        });
    });
}

fn current_id() -> Option<u64> {
    TRACE.with(|trace| {
        trace
            .borrow()
            .as_ref()
            .and_then(|trace| (trace.started.elapsed() <= TRACE_WINDOW).then_some(trace.id))
    })
}

/// Nested stage timings are inclusive; do not add parent and child durations.
pub(crate) struct Stage {
    id: Option<u64>,
    name: &'static str,
    started: Option<Instant>,
}

impl Stage {
    pub(crate) fn new(name: &'static str) -> Self {
        let id = current_id();
        Self {
            id,
            name,
            started: id.map(|_| Instant::now()),
        }
    }

    pub(crate) fn lap(&mut self, next: &'static str) {
        let Some(started) = self.started else { return };
        let elapsed = started.elapsed();
        if let Some(id) = self.id {
            TRACE.with(|trace| {
                if let Some(trace) = trace.borrow_mut().as_mut().filter(|t| t.id == id) {
                    if trace.stages.len() < MAX_STAGES {
                        trace.stages.push((self.name, elapsed));
                    }
                }
            });
        }
        self.name = next;
        self.started = Some(Instant::now());
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        self.lap("finished");
    }
}

pub(crate) struct Frame {
    started: Instant,
    last: Instant,
    ui_gap: Duration,
    stages: Option<Vec<(&'static str, Duration)>>,
}

impl Frame {
    pub(crate) fn new(previous_exit: Option<Instant>) -> Self {
        let now = Instant::now();
        let session = capture_session();
        TRACE.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.as_ref().is_some_and(|trace| {
                Some(trace.session) != session
                    || now.saturating_duration_since(trace.started) > TRACE_WINDOW
            }) {
                *slot = None;
            }
        });
        Self {
            started: now,
            last: now,
            ui_gap: previous_exit
                .map(|t| now.saturating_duration_since(t))
                .unwrap_or_default(),
            stages: session.map(|_| Vec::new()),
        }
    }

    pub(crate) fn checkpoint(&mut self, name: &'static str) {
        if let Some(stages) = &mut self.stages {
            stages.push((name, self.last.elapsed()));
            self.last = Instant::now();
        }
    }

    pub(crate) fn finish(self, loading: bool, items: usize, generation: usize) {
        if let Some((session, events)) = self.collect(loading, items, generation) {
            emit(session, events);
        }
    }

    fn collect(
        mut self,
        loading: bool,
        items: usize,
        generation: usize,
    ) -> Option<(u64, Vec<Event>)> {
        self.checkpoint("frame_tail");
        let Some(mut stages) = self.stages else {
            TRACE.with(|trace| *trace.borrow_mut() = None);
            return None;
        };
        let duration = self.started.elapsed();
        let snapshot = TRACE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let trace = slot.as_mut()?;
            // A late repaint after idle is not a frame inside the capture window.
            // Keep a frame which started in the window but itself stalled past it.
            if self.started.saturating_duration_since(trace.started) > TRACE_WINDOW
                || trace.generation.is_some_and(|g| g != generation)
            {
                *slot = None;
                return None;
            }
            trace.frames += 1;
            let completed = trace.last_loading && !loading;
            trace.last_loading = loading;
            stages.append(&mut trace.stages);
            let expired = trace.started.elapsed() > TRACE_WINDOW;
            let report = trace.frames <= 3
                || duration >= SLOW_FRAME
                || self.ui_gap >= SLOW_FRAME
                || completed
                || expired;
            let snapshot = report.then_some((
                trace.session,
                trace.id,
                trace.frames,
                trace.started.elapsed(),
                completed,
            ));
            if expired {
                *slot = None;
            }
            snapshot
        });
        let (session, id, frame, since_click, completed) = snapshot?;
        let mut events = vec![Event(
            "ui_frame",
            vec![
                field_u64("trace_id", id),
                field_u64("frame", frame),
                field_label("source", "sidebar_drive"),
                field_u64("generation", generation as u64),
                field_duration_ms("elapsed_since_click", since_click),
                field_duration_ms("duration", duration),
                field_duration_ms("ui_gap", self.ui_gap),
                field_bool("slow", duration >= SLOW_FRAME),
                field_bool("loading", loading),
                field_bool("load_completed", completed),
                field_u64("items", items as u64),
            ],
        )];
        for (name, duration) in stages {
            events.push(Event(
                "ui_stage",
                vec![
                    field_u64("trace_id", id),
                    field_u64("frame", frame),
                    field_label("stage", name),
                    field_duration_ms("duration", duration),
                ],
            ));
        }
        Some((session, events))
    }
}

/// Travels with the job: queue delay and worker time must not be mistaken for UI time.
pub(crate) struct Worker {
    id: u64,
    session: u64,
    queued: Instant,
    running: Option<Instant>,
    generation: usize,
    stage: &'static str,
    last: Instant,
    stages: Vec<(&'static str, Duration)>,
}

impl Worker {
    pub(crate) fn queued(generation: usize) -> Option<Self> {
        let id = current_id()?;
        let session = TRACE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let trace = slot.as_mut()?;
            // Do not attach unrelated later loads to the sidebar click.
            if trace.generation.is_some_and(|g| g != generation) {
                return None;
            }
            trace.generation = Some(generation);
            trace.last_loading = true;
            Some(trace.session)
        })?;
        let now = Instant::now();
        Some(Self {
            id,
            session,
            queued: now,
            running: None,
            generation,
            stage: "generation_guard",
            last: now,
            stages: Vec::new(),
        })
    }

    pub(crate) fn start(&mut self) {
        let now = Instant::now();
        self.running = Some(now);
        self.last = now;
        emit(
            self.session,
            vec![Event(
                "worker_started",
                vec![
                    field_u64("trace_id", self.id),
                    field_u64("generation", self.generation as u64),
                    field_duration_ms("queue_wait", now.saturating_duration_since(self.queued)),
                ],
            )],
        );
    }

    pub(crate) fn lap(&mut self, next: &'static str) {
        self.stages.push((self.stage, self.last.elapsed()));
        self.stage = next;
        self.last = Instant::now();
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Superseded queued jobs may be dropped while the UI holds the pool lock.
        // Never perform diagnostic file writes there.
        if self.running.is_none() {
            return;
        }
        self.lap("finished");
        let duration = self.running.map(|t| t.elapsed()).unwrap_or_default();
        let mut events = vec![Event(
            "worker_finished",
            vec![
                field_u64("trace_id", self.id),
                field_u64("generation", self.generation as u64),
                field_bool("started", self.running.is_some()),
                field_duration_ms("worker_duration", duration),
                field_duration_ms("total_since_queue", self.queued.elapsed()),
            ],
        )];
        for &(name, duration) in &self.stages {
            events.push(Event(
                "worker_stage",
                vec![
                    field_u64("trace_id", self.id),
                    field_u64("generation", self.generation as u64),
                    field_label("stage", name),
                    field_duration_ms("duration", duration),
                ],
            ));
        }
        emit(self.session, events);
    }
}

#[cfg(test)]
mod tests;
