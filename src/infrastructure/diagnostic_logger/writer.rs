//! Only this thread touches the diagnostic file. Event admission is bounded;
//! lifecycle commands bypass admission so closing a full session cannot be lost.
use super::{DiagnosticField, DiagnosticFile};
use log::Level;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_PENDING_BATCHES: usize = 64;
const MAX_PENDING_EVENTS: usize = 8192;
// Routine logging must not consume the space reserved for a stall's history.
const NORMAL_PENDING_BATCHES: usize = 56;
const NORMAL_PENDING_EVENTS: usize = 2048;
const FLUSH_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug)]
pub(super) struct Event {
    level: Level,
    component: &'static str,
    code: &'static str,
    fields: Vec<DiagnosticField>,
}

impl Event {
    pub(super) fn new(
        level: Level,
        component: &'static str,
        code: &'static str,
        mut fields: Vec<DiagnosticField>,
    ) -> Self {
        fields.push(DiagnosticField::U64(
            "captured_at_epoch_ms",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
        ));
        Self {
            level,
            component,
            code,
            fields,
        }
    }
}

enum Command {
    Open(u64, PathBuf, SystemTime),
    Events(u64, Vec<Event>),
    Close(u64),
    Flush(mpsc::SyncSender<()>),
    #[cfg(test)]
    Block(mpsc::SyncSender<()>, mpsc::Receiver<()>),
}

#[derive(Default)]
struct Counters {
    pending: AtomicUsize,
    dropped: AtomicU64,
    rejected: AtomicU64,
    failed: AtomicU64,
    pending_events: AtomicUsize,
    output_failed: AtomicBool,
    failure_reported: AtomicBool,
}

pub(super) struct Writer {
    sender: OnceLock<Option<mpsc::Sender<Command>>>,
    counters: Arc<Counters>,
}

impl Writer {
    pub(super) fn new() -> Self {
        Self {
            sender: OnceLock::new(),
            counters: Arc::new(Counters::default()),
        }
    }

    fn sender(&self) -> Option<&mpsc::Sender<Command>> {
        self.sender
            .get_or_init(|| {
                let (tx, rx) = mpsc::channel();
                let counters = self.counters.clone();
                std::thread::Builder::new()
                    .name("diagnostic-writer".into())
                    .spawn(move || run(rx, counters))
                    .ok()
                    .map(|_| tx)
            })
            .as_ref()
    }

    pub(super) fn open(
        &self,
        session: u64,
        path: PathBuf,
        since: SystemTime,
    ) -> Result<(), String> {
        self.sender()
            .ok_or("Diagnostic writer unavailable")?
            .send(Command::Open(session, path, since))
            .map_err(|_| "Diagnostic writer stopped".into())
    }

    pub(super) fn close(&self, session: u64) {
        if let Some(sender) = self.sender() {
            let _ = sender.send(Command::Close(session));
        }
    }

    pub(super) fn events(&self, session: u64, events: Vec<Event>) {
        if self.enqueue_events(session, events, false).is_err() {
            self.counters.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(super) fn critical_events(&self, session: u64, events: Vec<Event>) {
        if self.try_critical_events(session, events).is_err() {
            self.counters.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(super) fn try_critical_events(
        &self,
        session: u64,
        events: Vec<Event>,
    ) -> Result<(), Vec<Event>> {
        self.enqueue_events(session, events, true)
    }

    fn enqueue_events(
        &self,
        session: u64,
        events: Vec<Event>,
        critical: bool,
    ) -> Result<(), Vec<Event>> {
        let max_events = if critical {
            MAX_PENDING_EVENTS
        } else {
            NORMAL_PENDING_EVENTS
        };
        let max_batches = if critical {
            MAX_PENDING_BATCHES
        } else {
            NORMAL_PENDING_BATCHES
        };
        let count = events.len();
        if self
            .counters
            .pending_events
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                n.checked_add(count).filter(|total| *total <= max_events)
            })
            .is_err()
        {
            self.counters.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(events);
        }
        if self
            .counters
            .pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < max_batches).then_some(n + 1)
            })
            .is_err()
        {
            self.counters
                .pending_events
                .fetch_sub(count, Ordering::AcqRel);
            self.counters.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(events);
        }
        let rejected = match self.sender() {
            Some(tx) => match tx.send(Command::Events(session, events)) {
                Ok(()) => return Ok(()),
                Err(mpsc::SendError(Command::Events(_, events))) => events,
                Err(_) => unreachable!("only an event command was sent"),
            },
            None => events,
        };
        self.counters
            .pending_events
            .fetch_sub(count, Ordering::AcqRel);
        self.counters.pending.fetch_sub(1, Ordering::AcqRel);
        self.counters.rejected.fetch_add(1, Ordering::Relaxed);
        record_failure(&self.counters);
        Err(rejected)
    }

    pub(super) fn flush(&self, timeout: Duration) {
        let Some(Some(sender)) = self.sender.get() else {
            return;
        };
        let (tx, rx) = mpsc::sync_channel(1);
        if sender.send(Command::Flush(tx)).is_ok() {
            let _ = rx.recv_timeout(timeout);
        }
    }

    pub(super) fn failures(&self) -> u64 {
        self.counters.failed.load(Ordering::Relaxed)
    }
    pub(super) fn take_output_failure(&self) -> bool {
        self.counters.output_failed.load(Ordering::Acquire)
            && !self.counters.failure_reported.swap(true, Ordering::AcqRel)
    }
}

fn finish(file: &mut Option<(u64, DiagnosticFile)>, counters: &Counters) {
    if let Some((_, mut file)) = file.take() {
        if file
            .write_session_footer()
            .and_then(|_| file.flush())
            .is_err()
        {
            record_failure(counters);
        }
    }
}

fn run(rx: mpsc::Receiver<Command>, counters: Arc<Counters>) {
    let mut file: Option<(u64, DiagnosticFile)> = None;
    let mut last_flush = std::time::Instant::now();
    loop {
        match rx.recv_timeout(FLUSH_INTERVAL) {
            Ok(Command::Open(session, path, since)) => {
                finish(&mut file, &counters);
                counters.output_failed.store(false, Ordering::Release);
                counters.failure_reported.store(false, Ordering::Release);
                let result = path
                    .parent()
                    .ok_or_else(|| std::io::Error::other("Missing log directory"))
                    .and_then(std::fs::create_dir_all)
                    .and_then(|_| DiagnosticFile::open(path, since));
                match result {
                    Ok(opened) => file = Some((session, opened)),
                    Err(_) => {
                        record_failure(&counters);
                    }
                }
            }
            Ok(Command::Events(session, events)) => {
                counters.pending.fetch_sub(1, Ordering::AcqRel);
                counters
                    .pending_events
                    .fetch_sub(events.len(), Ordering::AcqRel);
                if let Some((_, output)) = file.as_mut().filter(|(current, _)| *current == session)
                {
                    let dropped = counters.dropped.swap(0, Ordering::Relaxed);
                    let rejected = counters.rejected.swap(0, Ordering::Relaxed);
                    if (dropped > 0 || rejected > 0)
                        && output
                            .write_event(
                                Level::Warn,
                                "diagnostic_writer",
                                "capture_overflow",
                                &[
                                    DiagnosticField::U64("dropped_batches", dropped),
                                    DiagnosticField::U64("rejected_batches", rejected),
                                ],
                            )
                            .is_err()
                    {
                        record_failure(&counters);
                    }
                    for event in events {
                        if output
                            .write_event(event.level, event.component, event.code, &event.fields)
                            .is_err()
                        {
                            record_failure(&counters);
                        }
                    }
                }
            }
            Ok(Command::Close(session)) => {
                if file
                    .as_ref()
                    .is_some_and(|(current, _)| *current == session)
                {
                    finish(&mut file, &counters);
                }
            }
            Ok(Command::Flush(done)) => {
                if let Some((_, file)) = file.as_mut() {
                    if file.flush().is_err() {
                        record_failure(&counters);
                    }
                }
                let _ = done.send(());
            }
            #[cfg(test)]
            Ok(Command::Block(started, release)) => {
                let _ = started.send(());
                let _ = release.recv();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if last_flush.elapsed() >= FLUSH_INTERVAL {
            if let Some((_, file)) = file.as_mut() {
                if file.flush().is_err() {
                    record_failure(&counters);
                }
            }
            last_flush = std::time::Instant::now();
        }
    }
    finish(&mut file, &counters);
}

fn record_failure(counters: &Counters) {
    counters.failed.fetch_add(1, Ordering::Relaxed);
    counters.output_failed.store(true, Ordering::Release);
}

#[cfg(test)]
mod tests;
