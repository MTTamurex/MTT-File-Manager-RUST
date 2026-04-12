use crate::ui::cache::FxHashSet;
use lru::LruCache;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum FolderSizeMessage {
    Progress {
        folder_path: PathBuf,
        total_size: u64,
    },
    Complete {
        folder_path: PathBuf,
        total_size: u64,
    },
    Cancelled {
        folder_path: PathBuf,
    },
}

pub struct FolderSizeState {
    pub req_sender: Sender<PathBuf>,
    pub res_receiver: Receiver<FolderSizeMessage>,
    pub cancel: Arc<AtomicBool>,
    pub cache: LruCache<PathBuf, u64>,
    pub loading: FxHashSet<PathBuf>,

    // ── Batch worker for list-view folder sizes ──
    /// Sender for background batch requests.
    pub batch_req_sender: Sender<PathBuf>,
    /// Receiver for batch results.
    pub batch_res_receiver: Receiver<FolderSizeMessage>,
    /// Shared cancel flag — set on navigation to abort in-flight scans.
    pub batch_cancel: Arc<AtomicBool>,
    /// Paths already sent to batch worker, awaiting response.
    pub batch_loading: FxHashSet<PathBuf>,
    /// Dedicated LRU cache for list-view folder sizes (larger capacity).
    pub batch_cache: LruCache<PathBuf, u64>,
}

impl FolderSizeState {
    /// Cancel all pending batch work and drain stale results.
    ///
    /// Call on every navigation or List→Grid switch to stop orphan
    /// slow-path scans from the previous folder.
    pub fn cancel_batch(&mut self) {
        // 1. Signal the worker to skip queued items and abort in-flight
        //    FindFirstFileExW scans (checked by calculate_folder_size_parallel).
        self.batch_cancel.store(true, Ordering::Release);

        // 2. Drain stale results so they don't leak into the new folder.
        while self.batch_res_receiver.try_recv().is_ok() {}

        // 3. Clear the dedup set so new requests for the same paths
        //    aren't incorrectly blocked.
        self.batch_loading.clear();

        // 4. Re-enable the worker for the next folder.
        self.batch_cancel.store(false, Ordering::Release);
    }
}
