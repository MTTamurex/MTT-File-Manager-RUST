//! Themed, snapshot-scoped emptying. No fallback or automatic deletion retries.
use super::super::{FileOperationRequest, FileOperationResult, SendHwnd};
use crate::infrastructure::copy_engine::{CopyRuntime, Outcome, OutcomeKind};
use crate::infrastructure::delete_engine::confirmation;
use crate::infrastructure::windows::recycle_bin;
use std::sync::{atomic::Ordering, mpsc::Sender, Arc};
mod identity;
mod sink;
mod tracking;

pub(super) fn try_handle(
    request: &FileOperationRequest,
    sender: &Sender<FileOperationResult>,
    runtime: &Arc<CopyRuntime>,
) -> bool {
    let FileOperationRequest::EmptyRecycleBin { hwnd } = request else {
        return false;
    };
    let Some(_running) = Running::acquire(runtime) else {
        return true;
    };
    // Refresh even when a COM call panics after making partial progress.
    let _refresh = RefreshOnDrop(sender);
    handle(*hwnd, runtime);
    true
}

struct Running<'a>(&'a CopyRuntime);

impl<'a> Running<'a> {
    fn acquire(runtime: &'a CopyRuntime) -> Option<Self> {
        runtime
            .empty_recycle_bin_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(runtime))
    }
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0
            .empty_recycle_bin_running
            .store(false, Ordering::Release);
    }
}

struct RefreshOnDrop<'a>(&'a Sender<FileOperationResult>);

impl Drop for RefreshOnDrop<'_> {
    fn drop(&mut self) {
        let _ = self.0.send(FileOperationResult::RecycleBinChanged);
    }
}

fn handle(hwnd: SendHwnd, runtime: &Arc<CopyRuntime>) {
    let job = runtime.start_delete_with_label(
        Vec::new(),
        true,
        Some(rust_i18n::t!("delete_engine.recycle_bin").to_string()),
    );
    let operation = &job.operation;
    let progress = operation.deletion.as_ref().unwrap();
    let items = match recycle_bin::enumerate_recycle_bin_shell_items(|| {
        operation.progress.is_cancelled()
    }) {
        Ok(items) => items,
        Err(error) => {
            let cancelled =
                operation.progress.is_cancelled() || tracking::is_cancellation(error.code());
            job.finish(Outcome {
                kind: if cancelled {
                    OutcomeKind::Cancelled
                } else {
                    OutcomeKind::Failed
                },
                message: (!cancelled).then(|| tracking::error_message(&error)),
            });
            return;
        }
    };
    let total = items.len();
    progress.total.store(total as u64, Ordering::Relaxed);
    if total == 0 {
        // Another process may already have emptied it. Do not leave a spurious
        // success card on the dashboard for a no-op.
        job.finish(Outcome {
            kind: OutcomeKind::Completed,
            message: None,
        });
        operation.close();
        return;
    }
    if !confirmation::ask_empty_recycle_bin(operation, total) || operation.progress.is_cancelled() {
        job.finish(Outcome {
            kind: OutcomeKind::Cancelled,
            message: None,
        });
        return;
    }

    let tracking = Arc::new(tracking::Tracking::new(operation.clone(), total));
    let sink = sink::global(tracking.clone());
    let result = recycle_bin::delete_recycle_bin_items_with_progress(
        hwnd.0,
        &items,
        &sink,
        |index, item| sink::item(tracking.clone(), index, item),
        || operation.progress.is_cancelled(),
    );
    job.finish(tracking.outcome(result));
}

#[cfg(test)]
mod tests;
