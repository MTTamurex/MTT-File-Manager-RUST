//! Count selected roots once, never Shell work units or recursive child callbacks.
use crate::infrastructure::copy_engine::{CopyOperation, Outcome, OutcomeKind};
use std::collections::BTreeMap;
use std::sync::{atomic::Ordering, Arc, Mutex};
use windows::core::{Error, Result, HRESULT};
use windows::Win32::Foundation::{ERROR_CANCELLED, E_ABORT, S_OK};
use windows::Win32::UI::Shell::{
    COPYENGINE_E_CANCELLED, COPYENGINE_E_USER_CANCELLED, COPYENGINE_S_ALREADY_DONE,
    COPYENGINE_S_COLLISIONRESOLVED, COPYENGINE_S_DONT_PROCESS_CHILDREN,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ItemState {
    Pending,
    Removed,
    Failed,
}

pub(super) struct Tracking {
    pub(super) operation: Arc<CopyOperation>,
    items: Mutex<Vec<ItemState>>,
    finish_error: Mutex<Option<HRESULT>>,
    callback_results: Mutex<BTreeMap<(i32, bool), usize>>,
}

impl Tracking {
    pub(super) fn new(operation: Arc<CopyOperation>, total: usize) -> Self {
        Self {
            operation,
            items: Mutex::new(vec![ItemState::Pending; total]),
            finish_error: Mutex::new(None),
            callback_results: Mutex::new(BTreeMap::new()),
        }
    }

    pub(super) fn record(&self, index: usize, is_root: bool, result: HRESULT) {
        *self
            .callback_results
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry((result.0, is_root))
            .or_default() += 1;
        if is_cancellation(result) {
            self.operation.progress.cancel();
            return;
        }
        // S_OK is not the only successful completion. Conversely, informational
        // codes (skip, retry, pending, etc.) prove neither removal nor failure.
        let completed = [
            S_OK,
            COPYENGINE_S_DONT_PROCESS_CHILDREN,
            COPYENGINE_S_ALREADY_DONE,
            COPYENGINE_S_COLLISIONRESOLVED,
        ]
        .contains(&result);
        if result.is_ok() && !completed {
            return;
        }
        let mut items = self.items.lock().unwrap_or_else(|e| e.into_inner());
        if items[index] != ItemState::Pending {
            return;
        }
        let progress = self.operation.deletion.as_ref().unwrap();
        if is_root && completed {
            items[index] = ItemState::Removed;
            progress.removed();
        } else if result.is_err() {
            // A failed child makes the selected folder incomplete, but counts
            // as one failed root even when more descendants subsequently fail.
            items[index] = ItemState::Failed;
            progress.failed();
        }
    }

    pub(super) fn finish_result(&self, result: HRESULT) {
        if is_cancellation(result) {
            self.operation.progress.cancel();
        } else if result.is_err() {
            *self.finish_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
        }
    }

    pub(super) fn outcome(&self, result: Result<bool>) -> Outcome {
        // Aggregate codes only: never log Shell item names, identities or paths.
        for (&(code, is_root), &count) in self
            .callback_results
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            log::info!(
                "[RecycleBin] Delete callbacks: hresult=0x{:08X}, selected_root={}, count={}",
                code as u32,
                is_root,
                count
            );
        }
        let execution_error = result.as_ref().err().map(|error| error.code());
        let finish_error = *self.finish_error.lock().unwrap_or_else(|e| e.into_inner());
        // A subsequent E_ABORT must not hide the actual error reported at
        // FinishOperations, even if cancellation races with that error.
        let error = execution_error
            .filter(|code| !is_cancellation(*code))
            .or(finish_error)
            .or(execution_error);
        let cancelled = self.operation.progress.is_cancelled()
            || error.is_some_and(is_cancellation)
            // Shell also aborts batches on errors: do not mask those failures
            // as a user cancellation merely because the aborted bit is set.
            || (matches!(result, Ok(true)) && error.is_none());
        let progress = self.operation.deletion.as_ref().unwrap();
        progress.set_current(None);
        // Missing callbacks do not prove a deletion error, nor success.
        let unconfirmed = !cancelled
            && self
                .items
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains(&ItemState::Pending);
        let failures = progress.failures.load(Ordering::Relaxed);
        let operation_error = error.filter(|code| !is_cancellation(*code));
        if failures > 0 || operation_error.is_some() || unconfirmed {
            let mut messages = Vec::new();
            if failures > 0 {
                messages.push(rust_i18n::t!("delete_engine.failed", count = failures).to_string());
            }
            if let Some(code) = operation_error {
                messages.push(error_message(&Error::from_hresult(code)));
            }
            if unconfirmed {
                messages
                    .push(rust_i18n::t!("operations.empty_recycle_bin_unconfirmed").to_string());
            }
            Outcome {
                kind: OutcomeKind::Failed,
                message: Some(messages.join("\n")),
            }
        } else if cancelled {
            Outcome {
                kind: OutcomeKind::Cancelled,
                message: None,
            }
        } else {
            Outcome {
                kind: OutcomeKind::Completed,
                message: None,
            }
        }
    }
}

pub(super) fn is_cancellation(result: HRESULT) -> bool {
    result == E_ABORT
        || result == COPYENGINE_E_USER_CANCELLED
        || result == COPYENGINE_E_CANCELLED
        || result == HRESULT::from_win32(ERROR_CANCELLED.0)
}

pub(super) fn error_message(error: &Error) -> String {
    rust_i18n::t!(
        "operations.empty_recycle_bin_failed",
        error = error.to_string()
    )
    .to_string()
}
