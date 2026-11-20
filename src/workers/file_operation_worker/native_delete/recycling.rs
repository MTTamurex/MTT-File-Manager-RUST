//! Stop Shell before an unrecyclable deletion. A separate, explicit MTT answer
//! is required before attempting the remaining contents of that root directly.
use crate::infrastructure::copy_engine::CopyOperation;
use crate::infrastructure::delete_engine::{
    confirmation::Kind, permanent, recycle, RemovalObserver,
};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use windows::Win32::Foundation::HWND;

const RECYCLE_BATCH_SIZE: usize = 128;

pub(super) fn run(
    paths: &[PathBuf],
    hwnd: HWND,
    operation: &Arc<CopyOperation>,
    observer: RemovalObserver,
    confirm: &impl Fn(&CopyOperation, Kind, &[PathBuf]) -> bool,
) -> bool {
    let mut recycle_batch =
        crate::infrastructure::windows::recycle_bin::RecycleCommitBatch::new(true);
    run_with_recycler(
        paths,
        operation,
        observer,
        confirm,
        |paths, operation, observer| {
            let progress = operation.deletion.as_ref().unwrap();
            let mut shell_paths = Vec::new();
            for (index, path) in paths.iter().enumerate() {
                if operation.progress.is_cancelled() {
                    let pending = shell_paths
                        .into_iter()
                        .chain(paths[index..].iter().cloned())
                        .collect();
                    return Ok(recycle::BatchOutcome {
                        pending,
                        cancelled: true,
                        ..Default::default()
                    });
                }
                progress.set_current(Some(path));
                if recycle_batch.try_recycle(path, || operation.progress.is_cancelled()) {
                    progress.recycled();
                    observer(path);
                } else {
                    if operation.progress.is_cancelled() {
                        let pending = shell_paths
                            .into_iter()
                            .chain(paths[index..].iter().cloned())
                            .collect();
                        return Ok(recycle::BatchOutcome {
                            pending,
                            cancelled: true,
                            ..Default::default()
                        });
                    }
                    shell_paths.push(path.clone());
                }
            }
            if shell_paths.is_empty() {
                Ok(recycle::BatchOutcome::default())
            } else {
                // A partial Shell operation can also change bin occupancy.
                recycle_batch.invalidate_after_shell();
                recycle::delete(&shell_paths, hwnd, operation, observer)
            }
        },
    )
}

fn run_with_recycler(
    paths: &[PathBuf],
    operation: &Arc<CopyOperation>,
    observer: RemovalObserver,
    confirm: &impl Fn(&CopyOperation, Kind, &[PathBuf]) -> bool,
    mut recycle: impl FnMut(
        &[PathBuf],
        Arc<CopyOperation>,
        RemovalObserver,
    ) -> windows::core::Result<recycle::BatchOutcome>,
) -> bool {
    let progress = operation.deletion.as_ref().unwrap();
    let mut pending: VecDeque<_> = paths.iter().cloned().collect();
    while !pending.is_empty() {
        if operation.progress.is_cancelled() {
            return false;
        }

        let count = pending.len().min(RECYCLE_BATCH_SIZE);
        let batch: Vec<_> = pending.drain(..count).collect();
        let failures_before = progress.failures.load(Ordering::Relaxed);
        let outcome = match recycle(&batch, operation.clone(), observer.clone()) {
            Ok(outcome) => outcome,
            Err(_) => {
                if !operation.progress.is_cancelled() {
                    progress.failed();
                }
                return false;
            }
        };
        if outcome.cancelled || operation.progress.is_cancelled() {
            operation.progress.cancel();
            return false;
        }
        if outcome.fatal {
            let failures_after = progress.failures.load(Ordering::Relaxed);
            if failures_after == failures_before {
                progress.failed();
            }
            return false;
        }

        let callback_failures = progress
            .failures
            .load(Ordering::Relaxed)
            .saturating_sub(failures_before) as usize;
        for path in &outcome.unavailable {
            progress.permanent_phase.store(false, Ordering::Relaxed);
            if !confirm(
                operation,
                Kind::RecycleUnavailable,
                std::slice::from_ref(path),
            ) || operation.progress.is_cancelled()
            {
                operation.progress.cancel();
                return false;
            }
            progress.permanent_phase.store(true, Ordering::Relaxed);
            progress
                .used_permanent_fallback
                .store(true, Ordering::Relaxed);
            let fallback = permanent::delete_with_observer(path, operation, &observer);
            progress.permanent_phase.store(false, Ordering::Relaxed);
            if let Err(error) = fallback {
                if error.kind() == std::io::ErrorKind::Interrupted {
                    operation.progress.cancel();
                    return false;
                }
                progress.failed();
            }
        }

        for _ in callback_failures..outcome.failed.len() {
            progress.failed();
        }

        if outcome.pending.len() == batch.len()
            && outcome.unavailable.is_empty()
            && outcome.failed.is_empty()
        {
            progress.failed();
            return false;
        }
        for path in outcome.pending.into_iter().rev() {
            pending.push_front(path);
        }
    }
    !operation.progress.is_cancelled()
}

#[cfg(test)]
mod tests;
