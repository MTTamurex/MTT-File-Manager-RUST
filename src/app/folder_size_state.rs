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
const FOLDER_SIZE_FAILURE_RETRY_DELAY: Duration = Duration::from_secs(30);
pub(crate) const PANEL_STALE_REVALIDATION_DELAY: Duration = Duration::from_millis(500);

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
    Failed {
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
    pub failed_until: HashMap<PathBuf, Instant>,
    /// Last complete values shown in the details panel after invalidation.
    pub panel_stale_cache: LruCache<PathBuf, FolderContentSummary>,
    pub panel_deferred_revalidation: HashMap<PathBuf, Instant>,

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
    pub fn preserve_panel_summary_for_deferred_revalidation(
        &mut self,
        folder_path: PathBuf,
        summary: FolderContentSummary,
        now: Instant,
    ) {
        if !summary.has_counts() {
            return;
        }

        self.panel_stale_cache.put(folder_path.clone(), summary);
        self.panel_deferred_revalidation
            .insert(folder_path, now + PANEL_STALE_REVALIDATION_DELAY);
        self.prune_panel_revalidations_without_stale();
    }

    pub fn reschedule_panel_revalidation_if_stale(&mut self, folder_path: &PathBuf, now: Instant) {
        if self.panel_stale_cache.contains(folder_path) {
            self.panel_deferred_revalidation
                .insert(folder_path.clone(), now + PANEL_STALE_REVALIDATION_DELAY);
        }
    }

    pub fn clear_panel_stale_summary(&mut self, folder_path: &PathBuf) {
        self.panel_stale_cache.pop(folder_path);
        self.panel_deferred_revalidation.remove(folder_path);
    }

    pub fn clear_failure(&mut self, folder_path: &PathBuf) {
        self.failed_until.remove(folder_path);
    }

    pub fn record_failure(&mut self, folder_path: PathBuf, now: Instant) {
        self.failed_until
            .insert(folder_path, now + FOLDER_SIZE_FAILURE_RETRY_DELAY);
    }

    pub fn is_failure_active(&mut self, folder_path: &PathBuf, now: Instant) -> bool {
        match self.failed_until.get(folder_path).copied() {
            Some(deadline) if deadline > now => true,
            Some(_) => {
                self.failed_until.remove(folder_path);
                false
            }
            None => false,
        }
    }

    pub fn summary_for_panel_render(
        &mut self,
        folder_path: &PathBuf,
        allow_stale: bool,
    ) -> (Option<FolderContentSummary>, bool) {
        let live_summary = self.cache.peek(folder_path).copied();
        let stale_summary = if allow_stale {
            self.panel_stale_cache.peek(folder_path).copied()
        } else {
            None
        };
        let use_stale = stale_summary.is_some()
            && match live_summary {
                Some(summary) => !summary.has_counts(),
                None => true,
            };

        let summary = if use_stale {
            stale_summary
        } else {
            live_summary
        };
        let loading = self.loading.contains(folder_path) && !use_stale;
        (summary, loading)
    }

    pub fn take_due_panel_revalidation(
        &mut self,
        now: Instant,
        current_path: &PathBuf,
    ) -> Option<PathBuf> {
        let deadline = self
            .panel_deferred_revalidation
            .get(current_path)
            .copied()?;
        if deadline > now {
            return None;
        }

        self.panel_deferred_revalidation.remove(current_path);
        if self.panel_stale_cache.contains(current_path) {
            Some(current_path.clone())
        } else {
            None
        }
    }

    fn prune_panel_revalidations_without_stale(&mut self) {
        let panel_stale_cache = &self.panel_stale_cache;
        self.panel_deferred_revalidation
            .retain(|path, _| panel_stale_cache.contains(path));
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroUsize;

    fn test_state() -> FolderSizeState {
        let (req_sender, _req_receiver) = std::sync::mpsc::channel();
        let (_res_sender, res_receiver) = std::sync::mpsc::channel();
        let (batch_req_sender, _batch_req_receiver) = std::sync::mpsc::channel();
        let (_batch_res_sender, batch_res_receiver) = std::sync::mpsc::channel();

        FolderSizeState {
            req_sender,
            res_receiver,
            cancel: Arc::new(AtomicBool::new(false)),
            cache: LruCache::new(NonZeroUsize::new(8).unwrap()),
            loading: FxHashSet::default(),
            failed_until: HashMap::new(),
            panel_stale_cache: LruCache::new(NonZeroUsize::new(8).unwrap()),
            panel_deferred_revalidation: HashMap::new(),
            batch_req_sender,
            batch_res_receiver,
            batch_cancel: Arc::new(AtomicBool::new(false)),
            batch_generation: Arc::new(AtomicU64::new(0)),
            batch_loading: FxHashSet::default(),
            batch_cache: LruCache::new(NonZeroUsize::new(8).unwrap()),
            pending_revalidation: HashMap::new(),
            pending_revalidation_last_prune: Instant::now(),
            batch_invalidation_epoch: HashMap::new(),
            batch_invalidation_last_prune: Instant::now(),
        }
    }

    #[test]
    fn panel_stale_summary_requires_complete_counts() {
        let mut state = test_state();
        let path = PathBuf::from(r"C:\data");
        let now = Instant::now();

        state.preserve_panel_summary_for_deferred_revalidation(
            path.clone(),
            FolderContentSummary::size_only(10),
            now,
        );
        assert!(state.panel_stale_cache.peek(&path).is_none());

        let summary = FolderContentSummary::complete(10, 2, 1);
        state.preserve_panel_summary_for_deferred_revalidation(path.clone(), summary, now);
        assert_eq!(state.panel_stale_cache.peek(&path).copied(), Some(summary));
    }

    #[test]
    fn panel_render_uses_stale_summary_without_loading_state() {
        let mut state = test_state();
        let path = PathBuf::from(r"C:\data");
        let stale = FolderContentSummary::complete(100, 12, 3);
        state.preserve_panel_summary_for_deferred_revalidation(path.clone(), stale, Instant::now());
        state
            .cache
            .put(path.clone(), FolderContentSummary::size_only(50));
        state.loading.insert(path.clone());

        let (summary, loading) = state.summary_for_panel_render(&path, true);

        assert_eq!(summary, Some(stale));
        assert!(!loading);
    }

    #[test]
    fn folder_size_failure_expires_after_retry_delay() {
        let mut state = test_state();
        let path = PathBuf::from(r"C:\data");
        let now = Instant::now();

        state.record_failure(path.clone(), now);

        assert!(state.is_failure_active(&path, now + Duration::from_secs(1)));
        assert!(!state.is_failure_active(
            &path,
            now + FOLDER_SIZE_FAILURE_RETRY_DELAY + Duration::from_millis(1)
        ));
    }

    #[test]
    fn panel_deferred_revalidation_waits_for_deadline() {
        let mut state = test_state();
        let path = PathBuf::from(r"C:\data");
        let other = PathBuf::from(r"C:\other");
        let now = Instant::now();

        state.preserve_panel_summary_for_deferred_revalidation(
            path.clone(),
            FolderContentSummary::complete(100, 12, 3),
            now,
        );

        assert_eq!(
            state.take_due_panel_revalidation(
                now + PANEL_STALE_REVALIDATION_DELAY - Duration::from_millis(1),
                &path,
            ),
            None
        );
        assert_eq!(
            state.take_due_panel_revalidation(
                now + PANEL_STALE_REVALIDATION_DELAY + Duration::from_millis(1),
                &other,
            ),
            None
        );
        assert_eq!(
            state.take_due_panel_revalidation(
                now + PANEL_STALE_REVALIDATION_DELAY + Duration::from_millis(1),
                &path,
            ),
            Some(path.clone())
        );
        assert_eq!(
            state.take_due_panel_revalidation(
                now + PANEL_STALE_REVALIDATION_DELAY + Duration::from_millis(2),
                &path,
            ),
            None
        );
    }
}
