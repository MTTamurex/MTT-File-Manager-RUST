//! Cooperative pause: callbacks keep their open file handles while sleeping.
use crate::Progress;
use std::io;
use std::sync::atomic::Ordering;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseState {
    Running,
    Pausing,
    Paused,
}

pub(crate) struct Control {
    state: Mutex<State>,
    wake: Condvar,
}

struct State {
    started: Instant,
    requested_at: Option<Instant>,
    paused_time: Duration,
    participants: usize,
    waiting: usize,
}

impl Default for Control {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                started: Instant::now(),
                requested_at: None,
                paused_time: Duration::ZERO,
                participants: 0,
                waiting: 0,
            }),
            wake: Condvar::new(),
        }
    }
}

impl Progress {
    pub fn cancel(&self) {
        // Use the same mutex as wait() to prevent a missed cancellation wakeup.
        let _state = self.control.state.lock().unwrap_or_else(|e| e.into_inner());
        self.cancel.store(true, Ordering::Release);
        self.control.wake.notify_all();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    pub fn pause(&self) {
        let mut state = self.control.state.lock().unwrap_or_else(|e| e.into_inner());
        if !self.is_cancelled() && state.requested_at.is_none() {
            state.requested_at = Some(Instant::now());
        }
    }

    pub fn resume(&self) {
        let mut state = self.control.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(start) = state.requested_at.take() {
            state.paused_time += start.elapsed();
        }
        self.control.wake.notify_all();
    }

    pub fn pause_state(&self) -> PauseState {
        let state = self.control.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.requested_at.is_none() {
            PauseState::Running
        } else if state.waiting >= state.participants {
            PauseState::Paused
        } else {
            PauseState::Pausing
        }
    }

    /// Elapsed time excluding requested pause intervals, including an ongoing one.
    pub fn active_elapsed(&self) -> Duration {
        let state = self.control.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        now.duration_since(state.started)
            .saturating_sub(state.paused_time)
            .saturating_sub(
                state
                    .requested_at
                    .map(|at| now.duration_since(at))
                    .unwrap_or_default(),
            )
    }

    pub(crate) fn participant(&self) -> Participant<'_> {
        self.control
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .participants += 1;
        Participant(self)
    }

    pub(crate) fn checkpoint(&self) -> io::Result<()> {
        self.checkpoint_with(|| {})
    }

    pub(crate) fn checkpoint_with(&self, on_pause: impl FnOnce()) -> io::Result<()> {
        let mut state = self.control.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.requested_at.is_some() && !self.is_cancelled() {
            on_pause();
            state.waiting += 1;
            while state.requested_at.is_some() && !self.is_cancelled() {
                state = self
                    .control
                    .wake
                    .wait(state)
                    .unwrap_or_else(|e| e.into_inner());
            }
            state.waiting -= 1;
        }
        if self.is_cancelled() {
            Err(io::Error::new(io::ErrorKind::Interrupted, "Copy cancelled"))
        } else {
            Ok(())
        }
    }
}

pub(crate) struct Participant<'a>(&'a Progress);

impl Drop for Participant<'_> {
    fn drop(&mut self) {
        self.0
            .control
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .participants -= 1;
    }
}

#[cfg(test)]
mod tests;
