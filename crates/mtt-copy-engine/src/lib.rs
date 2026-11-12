//! Copy-only engine inspired by BetterCopy's Win32 copying and storage profiling.
//! No Shell, clipboard, move or delete operations are exposed.
mod active_files;
mod control;
mod plan;
mod platform;
mod profile;
mod scheduler;
mod staging;
mod telemetry;
pub use control::PauseState;
pub use profile::PolicyInfo;
pub use telemetry::{Phase, Telemetry};

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

#[derive(Default)]
pub struct Progress {
    cancel: AtomicBool,
    control: control::Control,
    active: active_files::ActiveFiles,
    pub total_files: AtomicUsize,
    pub total_bytes: AtomicU64,
    pub files_copied: AtomicUsize,
    pub bytes_copied: AtomicU64,
    pub copying: AtomicBool,
    pub telemetry: Telemetry,
}

#[derive(Default, Debug)]
pub struct Summary {
    /// Includes partially populated roots so callers can refresh their views.
    pub destinations: Vec<PathBuf>,
    pub files_copied: usize,
    pub failures: usize,
    pub first_error: Option<io::Error>,
    pub cancelled: bool,
}

/// Refuse existing top-level targets, links/reparse points, and recursive copies
/// before writing anything. A failure after preflight never triggers a retry via
/// another engine. Completed files remain on cancellation; partial files do not.
pub fn copy(sources: &[PathBuf], destination: &Path, progress: &Progress) -> io::Result<Summary> {
    let _run = progress.telemetry.run();
    let coordinator = progress.participant();
    let plan = plan::build(sources, destination, progress)?;
    progress
        .total_files
        .store(plan.files.len(), Ordering::Relaxed);
    progress.total_bytes.store(plan.bytes, Ordering::Relaxed);
    let mut summary = Summary::default();
    for directory in &plan.directories {
        if progress.checkpoint().is_err() {
            summary.cancelled = true;
            return Ok(summary);
        }
        // Never merge an existing directory, including one created after preflight.
        let result = directory
            .parent()
            .ok_or_else(|| io::Error::other("Invalid parent"))
            .and_then(plan::check_path)
            .and_then(|()| std::fs::create_dir(directory));
        if let Err(error) = result {
            summary.failures += 1;
            summary.first_error = Some(error);
            return Ok(summary);
        }
        if plan.roots.contains(directory) {
            summary.destinations.push(directory.clone());
        }
    }

    progress.telemetry.set_phase(Phase::Profiling);
    let policy = profile::profile(sources, destination);
    let workers = policy.info.workers.min(plan.files.len()).max(1);
    progress.telemetry.set_policy(PolicyInfo {
        workers,
        large_workers: policy.info.large_workers.min(workers),
        ..policy.info
    });
    progress.telemetry.set_phase(Phase::Transferring);
    progress.copying.store(true, Ordering::Relaxed);
    let next_small = AtomicUsize::new(0);
    let next_large = AtomicUsize::new(0);
    let failures = AtomicUsize::new(0);
    let first_error = Mutex::new(None);
    let published = Mutex::new(Vec::new());
    let (small, large): (Vec<_>, Vec<_>) = plan
        .files
        .iter()
        .partition(|item| item.size < profile::LARGE_FILE_BYTES);
    drop(coordinator);
    std::thread::scope(|scope| {
        for worker in 0..workers {
            let small = &small;
            let large = &large;
            let next_small = &next_small;
            let next_large = &next_large;
            let failures = &failures;
            let first_error = &first_error;
            let published = &published;
            let policy = &policy;
            scope.spawn(move || {
                let _participant = progress.participant();
                // Reserve one worker for large files without exceeding the total
                // concurrency budget; all workers steal from the other queue.
                let queues = if worker == 0 {
                    [(large, next_large), (small, next_small)]
                } else {
                    [(small, next_small), (large, next_large)]
                };
                for (queue, next) in queues {
                    loop {
                        if progress.checkpoint().is_err() {
                            return;
                        }
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = queue.get(index) else {
                            break;
                        };
                        let Some(permit) = scheduler::global().acquire(
                            policy,
                            item.size >= profile::LARGE_FILE_BYTES,
                            progress,
                        ) else {
                            return;
                        };
                        match platform::copy_file_scheduled(item, progress, &permit) {
                            Ok(()) => {
                                progress.files_copied.fetch_add(1, Ordering::Relaxed);
                                published
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .push(item.destination.clone());
                            }
                            Err(_) if progress.cancel.load(Ordering::Relaxed) => return,
                            Err(error) => {
                                failures.fetch_add(1, Ordering::Relaxed);
                                first_error
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .get_or_insert(error);
                            }
                        }
                    }
                }
            });
        }
    });
    summary
        .destinations
        .extend(published.into_inner().unwrap_or_else(|e| e.into_inner()));
    summary.files_copied = progress.files_copied.load(Ordering::Relaxed);
    summary.failures = failures.load(Ordering::Relaxed);
    summary.first_error = first_error.into_inner().unwrap_or_else(|e| e.into_inner());
    summary.cancelled = progress.cancel.load(Ordering::Relaxed);
    Ok(summary)
}

fn check_cancel(progress: &Progress) -> io::Result<()> {
    progress.checkpoint()
}

#[cfg(test)]
mod tests;
