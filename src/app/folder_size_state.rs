use crate::ui::cache::FxHashSet;
use lru::LruCache;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

const INVALIDATION_EPOCH_PRUNE_INTERVAL: Duration = Duration::from_secs(2);
const INVALIDATION_EPOCH_PRUNE_THRESHOLD: usize = 1_024;
const FOLDER_SIZE_FAILURE_RETRY_DELAY: Duration = Duration::from_secs(30);
const FOLDER_SIZE_REVALIDATION_INITIAL_DELAY: Duration = Duration::from_secs(3);
pub(crate) const PANEL_STALE_REVALIDATION_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy)]
pub struct PendingFolderSizeRevalidation {
    deadline: Instant,
    stale_total_size: Option<u64>,
    release_batch_loading: bool,
}

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
        request_epoch: u64,
    },
    Complete {
        folder_path: PathBuf,
        summary: FolderContentSummary,
        request_epoch: u64,
    },
    Cancelled {
        folder_path: PathBuf,
        request_epoch: u64,
    },
    Failed {
        folder_path: PathBuf,
        request_epoch: u64,
    },
}

#[derive(Debug)]
pub struct FolderSizeRequest {
    pub folder_path: PathBuf,
    pub request_epoch: u64,
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
    pub req_sender: Sender<FolderSizeRequest>,
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
    /// Paths scheduled for one deferred re-invalidation.
    ///
    /// Handles the race condition where the search service's 2 s USN journal
    /// polling hasn't processed a file change before the client re-fetches
    /// the folder size, causing stale data to be permanently re-cached.
    pub pending_revalidation: HashMap<PathBuf, PendingFolderSizeRevalidation>,
    pub pending_revalidation_next_deadline: Option<Instant>,

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

        // 5. The worker clears the cancel flag only after it receives a request
        //    from the new generation. This makes cancellation observable by an
        //    in-flight scan instead of pulsing the flag too briefly.
        //
        // NOTE: pending_revalidation is intentionally NOT cleared here.
        // Revalidations are per-path and must survive navigation so they
        // can purge stale values that were re-cached from IPC or in-flight
        // scans that completed before the service updated its index.
    }

    pub fn should_prune_pending_revalidations(&self, now: Instant) -> bool {
        self.pending_revalidation_next_deadline
            .is_some_and(|deadline| deadline <= now)
    }

    pub fn schedule_revalidation(
        &mut self,
        folder_path: PathBuf,
        stale_total_size: Option<u64>,
        now: Instant,
    ) -> Duration {
        let deadline = now + FOLDER_SIZE_REVALIDATION_INITIAL_DELAY;
        self.pending_revalidation.insert(
            folder_path,
            PendingFolderSizeRevalidation {
                deadline,
                stale_total_size,
                release_batch_loading: false,
            },
        );
        self.pending_revalidation_next_deadline = Some(
            self.pending_revalidation_next_deadline
                .map_or(deadline, |current| current.min(deadline)),
        );
        FOLDER_SIZE_REVALIDATION_INITIAL_DELAY
    }

    pub fn schedule_revalidation_if_absent(
        &mut self,
        folder_path: PathBuf,
        now: Instant,
    ) -> Duration {
        let revalidation = self
            .pending_revalidation
            .entry(folder_path)
            .or_insert_with(|| PendingFolderSizeRevalidation {
                deadline: now + FOLDER_SIZE_REVALIDATION_INITIAL_DELAY,
                stale_total_size: None,
                release_batch_loading: true,
            });
        revalidation.release_batch_loading = true;
        let deadline = revalidation.deadline;
        self.pending_revalidation_next_deadline = Some(
            self.pending_revalidation_next_deadline
                .map_or(deadline, |current| current.min(deadline)),
        );
        deadline.saturating_duration_since(now)
    }

    pub fn cancel_revalidation(&mut self, folder_path: &PathBuf) {
        if self.pending_revalidation.remove(folder_path).is_some() {
            if self.pending_revalidation.is_empty() {
                self.pending_revalidation_next_deadline = None;
            }
        }
    }

    pub fn cancel_revalidations(&mut self, folder_paths: &[PathBuf]) {
        let mut changed = false;
        for folder_path in folder_paths {
            changed |= self.pending_revalidation.remove(folder_path).is_some();
        }
        if changed {
            if self.pending_revalidation.is_empty() {
                self.pending_revalidation_next_deadline = None;
            }
        }
    }

    pub fn cancel_revalidation_if_changed(&mut self, folder_path: &PathBuf, total_size: u64) {
        let changed = self
            .pending_revalidation
            .get(folder_path)
            .and_then(|entry| entry.stale_total_size)
            .is_some_and(|stale_total_size| stale_total_size != total_size);
        if changed {
            self.cancel_revalidation(folder_path);
        }
    }

    pub fn take_expired_revalidations(&mut self, now: Instant) -> Vec<(PathBuf, bool)> {
        let mut expired = Vec::new();
        self.pending_revalidation.retain(|path, revalidation| {
            let keep = revalidation.deadline > now;
            if !keep {
                expired.push((path.clone(), revalidation.release_batch_loading));
            }
            keep
        });
        self.pending_revalidation_next_deadline = self
            .pending_revalidation
            .values()
            .map(|entry| entry.deadline)
            .min();

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
mod tests;
