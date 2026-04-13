use crate::ui::cache::FxHashSet;
use lru::LruCache;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

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

/// Result from the batch folder-size worker.
///
/// Carries the `request_epoch` that was active when the request was sent,
/// allowing the consumer to detect stale results from scans that started
/// before a cache invalidation.
pub struct BatchSizeResult {
    pub folder_path: PathBuf,
    pub total_size: u64,
    /// Invalidation epoch copied from the request — compared against the
    /// current `batch_invalidation_epoch` to detect staleness.
    pub request_epoch: u64,
}

/// Batch request payload: (path, generation, invalidation_epoch).
pub type BatchSizeRequest = (PathBuf, u64, u64);

pub struct FolderSizeState {
    pub req_sender: Sender<PathBuf>,
    pub res_receiver: Receiver<FolderSizeMessage>,
    pub cancel: Arc<AtomicBool>,
    pub cache: LruCache<PathBuf, u64>,
    pub loading: FxHashSet<PathBuf>,

    // ── Batch worker for list-view folder sizes ──
    /// Sender for background batch requests.
    pub batch_req_sender: Sender<BatchSizeRequest>,
    /// Receiver for batch results (carries per-request epoch).
    pub batch_res_receiver: Receiver<BatchSizeResult>,
    /// Shared cancel flag — set on navigation to abort in-flight scans.
    pub batch_cancel: Arc<AtomicBool>,
    /// Monotonic generation counter — incremented on cancel to invalidate queued requests.
    pub batch_generation: Arc<AtomicU64>,
    /// Paths already sent to batch worker, awaiting response.
    pub batch_loading: FxHashSet<PathBuf>,
    /// Dedicated LRU cache for list-view folder sizes (larger capacity).
    pub batch_cache: LruCache<PathBuf, u64>,
    /// Paths scheduled for deferred re-invalidation.
    ///
    /// Handles the race condition where the search service's 2 s USN journal
    /// polling hasn't processed a file change before the client re-fetches
    /// the folder size, causing stale data to be permanently re-cached.
    /// Value = deadline after which the entry should be re-cleared.
    pub pending_revalidation: HashMap<PathBuf, Instant>,

    /// Per-path invalidation counter.  Incremented each time
    /// `invalidate_folder_size_cache(path)` is called.
    pub batch_invalidation_epoch: HashMap<PathBuf, u64>,
}

impl FolderSizeState {
    /// Cancel all pending batch work and drain stale results.
    ///
    /// Call on every navigation or List→Grid switch to stop orphan
    /// slow-path scans from the previous folder.
    pub fn cancel_batch(&mut self) {
        // 1. Bump generation so the worker discards all queued requests
        //    from the previous folder (they carry the old generation).
        self.batch_generation.fetch_add(1, Ordering::Release);

        // 2. Signal the worker to abort any in-flight FindFirstFileExW scan.
        self.batch_cancel.store(true, Ordering::Release);

        // 3. Drain stale results so they don't leak into the new folder.
        while self.batch_res_receiver.try_recv().is_ok() {}

        // 4. Clear the dedup set so new requests for the same paths
        //    aren't incorrectly blocked.
        self.batch_loading.clear();

        // 5. Re-enable the worker for the next folder.
        //
        // NOTE: pending_revalidation is intentionally NOT cleared here.
        // Revalidations are per-path and must survive navigation so they
        // can purge stale values that were re-cached from IPC or in-flight
        // scans that completed before the service updated its index.
        self.batch_cancel.store(false, Ordering::Release);
    }
}
