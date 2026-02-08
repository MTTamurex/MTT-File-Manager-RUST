use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::IoTimeoutResult;

pub(super) fn run_onedrive_timeout_operation<T, F>(
    path: &Path,
    timeout_ms: u64,
    poll_interval_ms: u64,
    op_name: &str,
    operation: F,
) -> IoTimeoutResult<T>
where
    T: Send + 'static,
    F: FnOnce(PathBuf) -> IoTimeoutResult<T> + Send + 'static,
{
    if super::is_app_minimized() {
        eprintln!(
            "[ONEDRIVE] App minimized - skipping {} for {:?}",
            op_name, path
        );
        return IoTimeoutResult::Timeout;
    }

    let current_threads = super::ACTIVE_TIMEOUT_THREADS.load(Ordering::SeqCst);
    if current_threads >= super::MAX_CONCURRENT_TIMEOUT_THREADS {
        eprintln!(
            "[ONEDRIVE] Thread limit reached ({}/{}), rejecting {} for {:?}",
            current_threads,
            super::MAX_CONCURRENT_TIMEOUT_THREADS,
            op_name,
            path
        );
        return IoTimeoutResult::Timeout;
    }

    let active_before = super::ACTIVE_TIMEOUT_THREADS.fetch_add(1, Ordering::SeqCst);
    eprintln!(
        "[ONEDRIVE] Active timeout threads: {} -> {}",
        active_before,
        active_before + 1
    );

    let path_buf = path.to_path_buf();
    let path_for_log = path_buf.clone();
    let (result_tx, result_rx) = mpsc::channel::<IoTimeoutResult<T>>();

    if super::onedrive_io_pool()
        .execute(move || {
            let _ = result_tx.send(operation(path_buf));
        })
        .is_err()
    {
        let active_after = super::ACTIVE_TIMEOUT_THREADS.fetch_sub(1, Ordering::SeqCst);
        eprintln!(
            "[ONEDRIVE] Active timeout threads: {} -> {}",
            active_after,
            active_after - 1
        );
        return IoTimeoutResult::Err(std::io::ErrorKind::Other);
    }

    let start = Instant::now();
    let timeout = Duration::from_millis(timeout_ms);
    let poll_interval = Duration::from_millis(poll_interval_ms.max(1));

    let result = loop {
        if super::is_app_minimized() {
            eprintln!(
                "[ONEDRIVE] App minimized during operation - aborting {} for {:?}",
                op_name, path_for_log
            );
            break IoTimeoutResult::Timeout;
        }

        if start.elapsed() >= timeout {
            eprintln!(
                "[ONEDRIVE TIMEOUT] {} exceeded {}ms for {:?}",
                op_name, timeout_ms, path_for_log
            );
            break IoTimeoutResult::Timeout;
        }

        let remaining = timeout.saturating_sub(start.elapsed());
        let wait_for = if remaining < poll_interval {
            remaining
        } else {
            poll_interval
        };

        match result_rx.recv_timeout(wait_for) {
            Ok(result) => break result,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break IoTimeoutResult::Err(std::io::ErrorKind::Other);
            }
        }
    };

    let active_after = super::ACTIVE_TIMEOUT_THREADS.fetch_sub(1, Ordering::SeqCst);
    eprintln!(
        "[ONEDRIVE] Active timeout threads: {} -> {}",
        active_after,
        active_after - 1
    );

    result
}

pub(super) fn metadata_with_timeout(
    path: &Path,
    timeout_ms: u64,
) -> IoTimeoutResult<std::fs::Metadata> {
    if !super::is_onedrive_path(path) {
        match std::fs::metadata(path) {
            Ok(m) => return IoTimeoutResult::Ok(m),
            Err(e) => return IoTimeoutResult::Err(e.kind()),
        }
    }

    let effective_timeout = if super::is_app_minimized() {
        super::ONEDRIVE_METADATA_TIMEOUT_MINIMIZED_MS
    } else {
        timeout_ms
    };
    run_onedrive_timeout_operation(path, effective_timeout, 1, "metadata()", move |path_buf| {
        match std::fs::metadata(&path_buf) {
            Ok(metadata) => IoTimeoutResult::Ok(metadata),
            Err(err) => IoTimeoutResult::Err(err.kind()),
        }
    })
}

pub(super) fn exists_with_timeout(path: &Path, timeout_ms: u64) -> IoTimeoutResult<bool> {
    if !super::is_onedrive_path(path) {
        return IoTimeoutResult::Ok(super::fast_path_exists(path));
    }

    let effective_timeout = if super::is_app_minimized() {
        super::ONEDRIVE_METADATA_TIMEOUT_MINIMIZED_MS
    } else {
        timeout_ms
    };
    run_onedrive_timeout_operation(path, effective_timeout, 1, "exists()", move |path_buf| {
        IoTimeoutResult::Ok(path_buf.exists())
    })
}

pub(super) fn onedrive_metadata(path: &Path) -> IoTimeoutResult<std::fs::Metadata> {
    metadata_with_timeout(path, super::ONEDRIVE_METADATA_TIMEOUT_MS)
}

pub(super) fn onedrive_exists(path: &Path) -> IoTimeoutResult<bool> {
    exists_with_timeout(path, super::ONEDRIVE_EXISTS_TIMEOUT_MS)
}
