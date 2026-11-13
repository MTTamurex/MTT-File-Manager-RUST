//! Worker/UI handshake. Only an explicit response to the current request permits
//! deletion; cancellation and stale responses fail closed.
use crate::infrastructure::copy_engine::CopyOperation;
use std::path::PathBuf;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Recycle,
    Permanent,
    RecycleUnavailable,
}

#[derive(Clone)]
pub struct Request {
    pub id: u64,
    pub kind: Kind,
    pub paths: Vec<PathBuf>,
}

#[derive(Default)]
struct State {
    waiting: bool,
    next: u64,
    pending: Option<Request>,
    answer: Option<Answer>,
    allow_remaining_fallbacks: bool,
}

struct Answer {
    allow: bool,
    apply_to_remaining: bool,
}

#[derive(Default)]
pub struct Confirmation {
    state: Mutex<State>,
    wake: Condvar,
}

impl Confirmation {
    pub fn pending(&self) -> Option<Request> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .clone()
    }

    pub fn respond(&self, id: u64, allow: bool) -> bool {
        self.respond_with_scope(id, allow, false)
    }

    /// Scope is limited to unrecyclable items in this operation. Recycling or
    /// permanent-delete confirmations cannot grant a fallback authorization.
    pub fn respond_with_scope(&self, id: u64, allow: bool, apply_to_remaining: bool) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state
            .pending
            .as_ref()
            .is_none_or(|request| request.id != id)
        {
            return false;
        }
        let fallback = state
            .pending
            .as_ref()
            .is_some_and(|request| request.kind == Kind::RecycleUnavailable);
        state.pending = None;
        state.answer = Some(Answer {
            allow,
            apply_to_remaining: allow && fallback && apply_to_remaining,
        });
        self.wake.notify_all();
        true
    }

    fn request(&self, kind: Kind, paths: &[PathBuf], control: &mtt_copy_engine::Progress) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if control.is_cancelled() || state.waiting {
            return false;
        }
        if kind == Kind::RecycleUnavailable && state.allow_remaining_fallbacks {
            return true;
        }
        state.waiting = true;
        let id = state.next;
        state.next = state.next.wrapping_add(1);
        state.answer = None;
        state.pending = Some(Request {
            id,
            kind,
            paths: paths.to_vec(),
        });
        loop {
            if control.is_cancelled() {
                state.pending = None;
                state.answer = None;
                state.waiting = false;
                return false;
            }
            if let Some(answer) = state.answer.take() {
                state.waiting = false;
                if answer.apply_to_remaining {
                    state.allow_remaining_fallbacks = true;
                }
                return answer.allow;
            }
            // Worker-only wait. The timeout observes cancellation even when the
            // progress control is cancelled outside the confirmation widget.
            state = self
                .wake
                .wait_timeout(state, Duration::from_millis(100))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

pub(crate) fn ask(operation: &CopyOperation, kind: Kind, paths: &[PathBuf]) -> bool {
    operation.deletion.as_ref().is_some_and(|deletion| {
        deletion
            .confirmation
            .request(kind, paths, &operation.progress)
    })
}

#[cfg(test)]
pub(crate) mod tests;
