use crate::ui::cache::FxHashSet;
use lru::LruCache;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

const PENDING_REVALIDATION_PRUNE_INTERVAL: Duration = Duration::from_millis(250);
const PENDING_REVALIDATION_PRUNE_THRESHOLD: usize = 500;
const INVALIDATION_EPOCH_PRUNE_INTERVAL: Duration = Duration::from_secs(2);
const INVALIDATION_EPOCH_PRUNE_THRESHOLD: usize = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderContentSummary {
    pub total_size: u64,
    pub file_count: Option<u64>,
    pub folder_count: Option<u64>,
}

impl FolderContentSummary {
    pub fn size_only(total_size: u64) -> Self {
        Self {
            total_size,
            file_count: None,
            folder_count: None,
        }
    }

    pub fn complete(total_size: u64, file_count: u64, folder_count: u64) -> Self {
        Self {
            total_size,
            file_count: Some(file_count),
            folder_count: Some(folder_count),
        }
    }

    pub fn has_counts(&self) -> bool {
        self.file_count.is_some() && self.folder_count.is_some()
    }

    pub fn with_total_size(self, total_size: u64) -> Self {
        Self { total_size, ..self }
    }
}

#[derive(Debug, Clone)]
pub enum FolderSizeMessage {
    Progress {
        folder_path: PathBuf,
        summary: FolderContentSummary,
    },
    Complete {
        folder_path: PathBuf,
        summary: FolderContentSummary,
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
    pub total_size: Option<u64>,
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
    pub cache: LruCache<PathBuf, FolderContentSummary>,
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
    pub pending_revalidation_last_prune: Instant,

    /// Per-path invalidation counter.  Incremented each time
    /// `invalidate_folder_size_cache(path)` is called.
    pub batch_invalidation_epoch: HashMap<PathBuf, u64>,
    pub batch_invalidation_last_prune: Instant,
}

impl FolderSizeState {
    /// Cancel all pending batch work and drain stale results.
    ///
    /// Call on every navigation or List→Grid switch to stop orphan
    /// slow-path scans from the previous folder.
    pub fn cancel_batch(&mut self) {
        // 0. Abort any in-flight single-folder full-tree scan so it
        //    doesn't keep running after the user navigates away.
        self.cancel.store(true, Ordering::Release);

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

    pub fn should_prune_pending_revalidations(&self, now: Instant) -> bool {
        !self.pending_revalidation.is_empty()
            && (self.pending_revalidation.len() > PENDING_REVALIDATION_PRUNE_THRESHOLD
                || now.duration_since(self.pending_revalidation_last_prune)
                    >= PENDING_REVALIDATION_PRUNE_INTERVAL)
    }

    pub fn take_expired_revalidations(&mut self, now: Instant) -> Vec<PathBuf> {
        self.pending_revalidation_last_prune = now;

        let mut expired = Vec::new();
        self.pending_revalidation.retain(|path, deadline| {
            let keep = *deadline > now;
            if !keep {
                expired.push(path.clone());
            }
            keep
        });

        expired
    }

    pub fn should_prune_invalidation_epochs(&self, now: Instant) -> bool {
        !self.batch_invalidation_epoch.is_empty()
            && (self.batch_invalidation_epoch.len() > INVALIDATION_EPOCH_PRUNE_THRESHOLD
                || now.duration_since(self.batch_invalidation_last_prune)
                    >= INVALIDATION_EPOCH_PRUNE_INTERVAL)
    }

    pub fn prune_stale_invalidation_epochs(&mut self, now: Instant) {
        self.batch_invalidation_last_prune = now;

        let loading = &self.loading;
        let batch_loading = &self.batch_loading;
        let cache = &self.cache;
        let batch_cache = &self.batch_cache;
        let pending_revalidation = &self.pending_revalidation;

        self.batch_invalidation_epoch.retain(|path, _| {
            loading.contains(path)
                || batch_loading.contains(path)
                || cache.contains(path)
                || batch_cache.contains(path)
                || pending_revalidation.contains_key(path)
        });
    }
}
