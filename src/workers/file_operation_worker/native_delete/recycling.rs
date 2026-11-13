//! Stop Shell before an unrecyclable deletion. A separate, explicit MTT answer
//! is required before attempting the remaining contents of that root directly.
use crate::infrastructure::copy_engine::CopyOperation;
use crate::infrastructure::delete_engine::{
    confirmation::Kind, permanent, recycle, RemovalObserver,
};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use windows::Win32::Foundation::HWND;

pub(super) fn run(
    paths: &[PathBuf],
    hwnd: HWND,
    operation: &Arc<CopyOperation>,
    observer: RemovalObserver,
    confirm: &impl Fn(&CopyOperation, Kind, &[PathBuf]) -> bool,
) -> bool {
    run_with_recycler(
        paths,
        operation,
        observer,
        confirm,
        |path, operation, observer| {
            recycle::delete(std::slice::from_ref(path), hwnd, operation, observer)
        },
    )
}

fn run_with_recycler(
    paths: &[PathBuf],
    operation: &Arc<CopyOperation>,
    observer: RemovalObserver,
    confirm: &impl Fn(&CopyOperation, Kind, &[PathBuf]) -> bool,
    recycle: impl Fn(
        &PathBuf,
        Arc<CopyOperation>,
        RemovalObserver,
    ) -> windows::core::Result<recycle::Status>,
) -> bool {
    let progress = operation.deletion.as_ref().unwrap();
    for path in paths {
        progress.permanent_phase.store(false, Ordering::Relaxed);
        if operation.progress.is_cancelled() {
            return false;
        }
        let result = recycle(path, operation.clone(), observer.clone());
        match result {
            Ok(recycle::Status::Completed) => {}
            Ok(recycle::Status::Unavailable) => {
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
                if let Err(error) = permanent::delete_with_observer(path, operation, &observer) {
                    if error.kind() != std::io::ErrorKind::Interrupted {
                        progress.failed();
                    }
                }
            }
            Ok(recycle::Status::Cancelled) => {
                operation.progress.cancel();
                return false;
            }
            Err(_) => {
                if !operation.progress.is_cancelled() {
                    progress.failed();
                }
                return false;
            }
        }
    }
    !operation.progress.is_cancelled()
}

#[cfg(test)]
mod tests;
