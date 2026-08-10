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

/// Everything a folder-load pipeline run needs. Sent through the mailbox;
/// all fields are cheap clones/Arcs.
pub(crate) struct FolderLoadJob {
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

/// Fixed-size worker pool with an unbounded latest-wins mailbox.
///
/// The mailbox itself cannot grow without bound in practice: each navigation
/// submits exactly one job and the generation guard makes stale jobs exit
/// immediately when picked up.
pub(crate) struct FolderLoadPool {
    tx: mpsc::Sender<FolderLoadJob>,
}

impl FolderLoadPool {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<FolderLoadJob>();
        let rx = Arc::new(std::sync::Mutex::new(rx));

        for worker_id in 0..FOLDER_LOAD_POOL_WORKERS {
            let rx = Arc::clone(&rx);
            let spawn_result = std::thread::Builder::new()
                .name(format!("folder-load-{worker_id}"))
                .spawn(move || {
                    while let Ok(job) = {
                        let guard = rx.lock().expect("folder-load mailbox mutex poisoned");
                        guard.recv()
                    } {
                        crate::app::operations::folder_loading::run_folder_load_pipeline(job);
                    }
                });

            if let Err(error) = spawn_result {
                log::error!(
                    "[FOLDER-LOAD-POOL] Failed to spawn worker {worker_id}: {}",
                    error
                );
            }
        }

        Self { tx }
    }

    /// Submit a load job. If all pool workers are dead (spawn failure at
    /// startup), surface the same load-failure the old spawn-error path used.
    pub fn submit(&self, job: FolderLoadJob) {
        if let Err(std::sync::mpsc::SendError(job)) = self.tx.send(job) {
            log::error!("[FOLDER-LOAD-POOL] All workers unavailable; reporting load failure");
            let _ = job.folder_load_failure_sender.send((
                job.my_gen,
                FolderLoadError::other(
                    std::path::PathBuf::from(&job.current_path),
                    "Folder load workers are unavailable".to_string(),
                ),
            ));
            job.ctx.request_repaint();
        }
    }
}

impl Default for FolderLoadPool {
    fn default() -> Self {
        Self::new()
    }
}
