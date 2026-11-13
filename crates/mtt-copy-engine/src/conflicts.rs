//! Per-operation conflict decisions. Workers wait without holding disk permits;
//! remembered answers never escape this Progress instance.
use crate::Progress;
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConflictKind {
    File,
    Directory,
    TypeMismatch,
    SamePath,
    Queued,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictAction {
    Replace,
    Skip,
    KeepBoth,
    Merge,
    Cancel,
}

#[derive(Clone, Debug)]
pub struct Conflict {
    pub id: u64,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub kind: ConflictKind,
}

impl Conflict {
    pub fn allows(&self, action: ConflictAction) -> bool {
        match action {
            ConflictAction::Replace => self.kind == ConflictKind::File,
            ConflictAction::Merge => self.kind == ConflictKind::Directory,
            _ => true,
        }
    }
}

#[derive(Default)]
struct State {
    next: u64,
    busy: bool,
    pending: Option<Conflict>,
    answer: Option<(ConflictAction, bool)>,
    remembered: HashMap<ConflictKind, ConflictAction>,
}

#[derive(Default)]
pub struct Conflicts {
    state: Mutex<State>,
    wake: Condvar,
}

impl Conflicts {
    pub fn pending(&self) -> Option<Conflict> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .clone()
    }

    pub fn respond(&self, id: u64, action: ConflictAction, apply_to_all: bool) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(request) = state.pending.as_ref() else {
            return false;
        };
        if request.id != id || !request.allows(action) {
            return false;
        }
        state.pending = None;
        state.answer = Some((action, apply_to_all));
        self.wake.notify_all();
        true
    }

    pub(crate) fn ask(
        &self,
        mut request: Conflict,
        progress: &Progress,
    ) -> io::Result<ConflictAction> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        while state.busy && !progress.is_cancelled() {
            state = self
                .wake
                .wait_timeout(state, Duration::from_millis(100))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        if progress.is_cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if let Some(action) = state.remembered.get(&request.kind) {
            return Ok(*action);
        }
        request.id = state.next;
        state.next = state.next.wrapping_add(1);
        let kind = request.kind;
        state.busy = true;
        state.pending = Some(request);
        loop {
            if progress.is_cancelled() {
                state.pending = None;
                state.answer = None;
                state.busy = false;
                self.wake.notify_all();
                return Err(io::ErrorKind::Interrupted.into());
            }
            if let Some((action, remember)) = state.answer.take() {
                state.busy = false;
                if action == ConflictAction::Cancel {
                    progress.cancel();
                } else if remember {
                    state.remembered.insert(kind, action);
                }
                self.wake.notify_all();
                return if action == ConflictAction::Cancel {
                    Err(io::ErrorKind::Interrupted.into())
                } else {
                    Ok(action)
                };
            }
            state = self
                .wake
                .wait_timeout(state, Duration::from_millis(100))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

#[cfg(test)]
mod tests;
