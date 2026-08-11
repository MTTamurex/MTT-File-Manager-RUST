//! EST-02: bounded persistent pool for folder-load pipelines.
//!
//! Previously every navigation spawned a detached "folder-load-pipeline"
//! thread. On volumes whose reads hang in the kernel (dead SMB share, stalled
//! FUSE driver, yanked removable media) those threads never terminate, so
//! repeated navigation across such paths accumulated one blocked thread —
//! plus its Arc'd cache/DB/sender clones — per navigation, without bound.
//!
//! The pool keeps a fixed number of workers. Stale jobs are discarded by the
//! existing generation guard as soon as a worker picks them up, so latest-wins
//! semantics are preserved. If every worker happens to be blocked inside
//! kernel I/O, new jobs wait in the mailbox instead of spawning more threads:
//! thread/handle/memory growth is capped at the pool size.

use crate::app::state::FolderLoadError;
use crate::domain::file_entry::FileEntry;
use crate::infrastructure::app_state_db::AppStateDb;
use crate::infrastructure::directory_cache::DirectoryCache;
use crate::infrastructure::directory_dirty_registry::DirectoryDirtyRegistry;
use crate::infrastructure::directory_index::DirectoryIndex;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use eframe::egui;
use std::sync::atomic::AtomicUsize;
use std::sync::{mpsc, Arc};

/// Pool size: covers both dual-panel lanes plus headroom for workers blocked
/// inside kernel I/O on unresponsive volumes.
pub(crate) const FOLDER_LOAD_POOL_WORKERS: usize = 4;
const FOLDER_LOAD_QUEUE_CAPACITY: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FolderLoadLane {
    Active,
    Inactive,
}

/// Everything a folder-load pipeline run needs. Sent through the mailbox;
/// all fields are cheap clones/Arcs.
pub(crate) struct FolderLoadJob {
    pub lane: FolderLoadLane,
    pub my_gen: usize,
    pub gen_clone: Arc<AtomicUsize>,
    pub current_path: String,
    pub force_refresh: bool,
    pub file_entry_sender: mpsc::Sender<(usize, Vec<FileEntry>)>,
    pub folder_load_failure_sender: mpsc::Sender<(usize, FolderLoadError)>,
    pub ctx: egui::Context,
    pub disk_cache: Arc<ThumbnailDiskCache>,
    pub app_state_db: Arc<AppStateDb>,
    pub directory_cache: Arc<DirectoryCache>,
    pub directory_dirty_registry: Arc<DirectoryDirtyRegistry>,
    pub directory_index_opt: Option<Arc<DirectoryIndex>>,
    pub show_hidden: bool,
}

/// Fixed-size worker pool with a bounded mailbox. Generation checks discard
/// stale queued work as workers become available, while the capacity prevents
/// blocked filesystem calls from turning repeated navigation into unbounded
/// memory growth.
pub(crate) struct FolderLoadPool {
    tx: crossbeam_channel::Sender<FolderLoadJob>,
    rx: crossbeam_channel::Receiver<FolderLoadJob>,
    live_workers: Arc<AtomicUsize>,
    submit_lock: parking_lot::Mutex<()>,
}

impl FolderLoadPool {
    pub fn new() -> Self {
        let (tx, rx) = crossbeam_channel::bounded::<FolderLoadJob>(FOLDER_LOAD_QUEUE_CAPACITY);

        let live_workers = Arc::new(AtomicUsize::new(0));
        for worker_id in 0..FOLDER_LOAD_POOL_WORKERS {
            let rx = rx.clone();
            let worker_count = Arc::clone(&live_workers);
            worker_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let spawn_result = std::thread::Builder::new()
                .name(format!("folder-load-{worker_id}"))
                .spawn(move || {
                    struct WorkerGuard(Arc<AtomicUsize>);
                    impl Drop for WorkerGuard {
                        fn drop(&mut self) {
                            self.0.fetch_sub(1, std::sync::atomic::Ordering::Release);
                        }
                    }
                    let _guard = WorkerGuard(worker_count);
                    while let Ok(job) = rx.recv() {
                        crate::app::operations::folder_loading::run_folder_load_pipeline(job);
                    }
                });

            if let Err(error) = spawn_result {
                log::error!(
                    "[FOLDER-LOAD-POOL] Failed to spawn worker {worker_id}: {}",
                    error
                );
                live_workers.fetch_sub(1, std::sync::atomic::Ordering::Release);
            }
        }

        Self {
            tx,
            rx,
            live_workers,
            submit_lock: parking_lot::Mutex::new(()),
        }
    }

    /// Submit without blocking the UI, retaining only the latest queued request
    /// for each panel lane. Running requests still rely on generation checks.
    pub fn submit(&self, job: FolderLoadJob) {
        if self.live_workers.load(std::sync::atomic::Ordering::Acquire) == 0 {
            report_load_failure(job, "Folder load workers are unavailable");
            return;
        }

        let _submit_guard = self.submit_lock.lock();
        let mut retained = Vec::new();
        while let Ok(queued) = self.rx.try_recv() {
            if queued.lane != job.lane {
                retained.push(queued);
            }
        }

        for queued in retained {
            if let Err(error) = self.tx.try_send(queued) {
                let queued = match error {
                    crossbeam_channel::TrySendError::Full(queued)
                    | crossbeam_channel::TrySendError::Disconnected(queued) => queued,
                };
                report_load_failure(queued, "Folder load workers are unavailable");
            }
        }

        if let Err(error) = self.tx.try_send(job) {
            let job = match error {
                crossbeam_channel::TrySendError::Full(job)
                | crossbeam_channel::TrySendError::Disconnected(job) => job,
            };
            report_load_failure(job, "Folder load workers are unavailable");
        }
    }
}

fn report_load_failure(job: FolderLoadJob, message: &str) {
    log::error!("[FOLDER-LOAD-POOL] {message}; reporting load failure");
    let _ = job.folder_load_failure_sender.send((
        job.my_gen,
        FolderLoadError::other(
            std::path::PathBuf::from(&job.current_path),
            message.to_string(),
        ),
    ));
    job.ctx.request_repaint();
}

impl Default for FolderLoadPool {
    fn default() -> Self {
        Self::new()
    }
}
