//! Priority thumbnail queue with HDD/SSD optimization
//!
//! Groups requests by directory on HDDs to minimize seek times.

use crate::infrastructure::io_priority::{self, IOPriority};
use crate::workers::thumbnail::types::{ThumbnailRequest, ThumbnailRequestSource};
use parking_lot::{Condvar, Mutex};
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SLOW_QUEUE_WAIT_THRESHOLD: Duration = Duration::from_secs(2);

/// Queue state with directory-grouped requests for HDD optimization
struct QueueState {
    /// Requests grouped by parent directory (for HDD locality optimization).
    ///
    /// PERF-03: every bucket is kept sorted by the pop ordering
    /// (`item_sort_key` → `directory_index` → `seq`), maintained
    /// incrementally on push/merge/promote. This replaces the previous
    /// O(M log M) `sort_by` executed on every HDD push and the O(M) full
    /// scans executed on every `pop`, both under the shared queue mutex.
    by_directory: FxHashMap<PathBuf, Vec<ThumbnailRequest>>,

    /// Quick lookup to prevent duplicates
    pending: FxHashSet<PathBuf>,

    /// Per-drive storage class cache (true = SSD, false = HDD)
    drive_is_ssd: FxHashMap<PathBuf, bool>,

    /// Current directory being processed (for HDD locality)
    current_directory: Option<PathBuf>,

    /// Shutdown flag
    shutdown: bool,

    /// PERF-03: number of pending `Normal`-source requests. The effective
    /// priority key of `BulkScan` requests depends on whether ANY normal
    /// request exists (bulk work is demoted while visible work is queued),
    /// so tracking the count lets mutations detect the two moments where
    /// bucket ordering must be re-established instead of scanning on every
    /// pop.
    normal_count: usize,

    /// PERF-03: number of pending `BulkScan`-source requests (used to skip
    /// the re-sort on normal-count transitions when no bulk work exists).
    bulk_count: usize,

    /// PERF-03: monotonic enqueue counter. Stable FIFO tie-break for
    /// requests with identical effective priority and directory index
    /// (replaces position-based tie-breaking, which depended on bucket
    /// arrangement).
    next_seq: u64,
}

pub struct PriorityThumbnailQueue {
    state: Mutex<QueueState>,
    condvar: Condvar,
}

impl Default for PriorityThumbnailQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl PriorityThumbnailQueue {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(QueueState {
                by_directory: FxHashMap::default(),
                pending: FxHashSet::default(),
                drive_is_ssd: FxHashMap::default(),
                current_directory: None,
                shutdown: false,
                normal_count: 0,
                bulk_count: 0,
                next_seq: 0,
            }),
            condvar: Condvar::new(),
        }
    }

    pub fn shutdown(&self) {
        {
            let mut state = self.state.lock();
            state.shutdown = true;
        }
        self.condvar.notify_all();
    }

    /// Returns the number of pending requests in the queue
    pub fn pending_count(&self) -> usize {
        self.state.lock().pending.len()
    }

    /// Clears stale normal thumbnail requests without touching drive profiling
    /// state. Bulk-scan requests are preserved so navigation does not strand
    /// the bulk progress counters after `total` has already been incremented.
    pub fn clear_pending(&self) -> usize {
        self.clear_pending_except_paths_internal(None)
    }

    /// Clears stale normal thumbnail requests while preserving requests for
    /// paths that are still visible in another panel.
    pub fn clear_pending_except_paths(&self, keep_paths: &FxHashSet<PathBuf>) -> usize {
        self.clear_pending_except_paths_internal(Some(keep_paths))
    }

    fn clear_pending_except_paths_internal(
        &self,
        keep_paths: Option<&FxHashSet<PathBuf>>,
    ) -> usize {
        let mut state = self.state.lock();

        let before = state.pending.len();
        state.by_directory.retain(|_, items| {
            items.retain_mut(|request| {
                if keep_paths.is_some_and(|paths| paths.contains(&request.path)) {
                    return true;
                }

                if !request.track_bulk_progress {
                    return false;
                }

                request.source = ThumbnailRequestSource::BulkScan;
                request.directory_index = None;
                if let Some(priority) = request.bulk_priority {
                    request.priority = priority;
                }
                true
            });

            !items.is_empty()
        });

        state.pending.clear();
        let retained_paths: Vec<PathBuf> = state
            .by_directory
            .values()
            .flat_map(|items| items.iter().map(|request| request.path.clone()))
            .collect();
        state.pending.extend(retained_paths);

        if state
            .current_directory
            .as_ref()
            .is_some_and(|dir| !state.by_directory.contains_key(dir))
        {
            state.current_directory = None;
        }

        // PERF-03: retained bulk entries may have had their priority/source
        // restored above, which changes their sort keys. Recount and re-sort.
        // This runs once per navigation, not per request.
        Self::recount_and_resort(&mut state);

        before.saturating_sub(state.pending.len())
    }

    /// Push a thumbnail request with the new IOPriority system
    pub fn push(
        &self,
        path: PathBuf,
        gen: usize,
        request_size: u32,
        priority: IOPriority,
        modified: u64,
    ) {
        self.push_with_epoch(path, gen, request_size, priority, modified, 0);
    }

    pub fn push_with_epoch(
        &self,
        path: PathBuf,
        gen: usize,
        request_size: u32,
        priority: IOPriority,
        modified: u64,
        request_epoch: u64,
    ) {
        self.push_with_index_and_epoch(
            path,
            gen,
            request_size,
            priority,
            None,
            modified,
            request_epoch,
        );
    }

    pub fn push_with_index(
        &self,
        path: PathBuf,
        gen: usize,
        request_size: u32,
        priority: IOPriority,
        directory_index: Option<usize>,
        modified: u64,
    ) {
        self.push_with_index_and_epoch(
            path,
            gen,
            request_size,
            priority,
            directory_index,
            modified,
            0,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_with_index_and_epoch(
        &self,
        path: PathBuf,
        gen: usize,
        request_size: u32,
        priority: IOPriority,
        directory_index: Option<usize>,
        modified: u64,
        request_epoch: u64,
    ) {
        self.push_with_index_and_source(
            path,
            gen,
            request_size,
            priority,
            directory_index,
            modified,
            request_epoch,
            ThumbnailRequestSource::Normal,
            None,
        );
    }

    pub fn push_bulk_scan(
        &self,
        path: PathBuf,
        gen: usize,
        request_size: u32,
        priority: IOPriority,
        modified: u64,
        bulk_session: u64,
    ) {
        self.push_with_index_and_source(
            path,
            gen,
            request_size,
            priority,
            None,
            modified,
            0,
            ThumbnailRequestSource::BulkScan,
            Some(bulk_session),
        );
    }

    pub fn promote_pending_to_interactive(
        &self,
        path: &Path,
        gen: usize,
        request_size: u32,
        directory_index: usize,
        modified: u64,
        request_epoch: u64,
    ) -> bool {
        let parent = path.parent().unwrap_or(path).to_path_buf();
        let promoted = {
            let mut state = self.state.lock();
            let had_normal_before = state.normal_count > 0;

            let Some(items) = state.by_directory.get_mut(&parent) else {
                return false;
            };

            let Some(existing_index) = items.iter().position(|req| req.path.as_path() == path)
            else {
                return false;
            };

            let was_normal = matches!(items[existing_index].source, ThumbnailRequestSource::Normal);

            {
                let existing = &mut items[existing_index];
                existing.priority = IOPriority::Interactive;
                existing.size = existing.size.max(request_size);
                existing.generation = existing.generation.max(gen);
                existing.directory_index = Some(directory_index);
                if modified > 0 && (existing.modified == 0 || modified > existing.modified) {
                    existing.modified = modified;
                }
                existing.request_epoch = existing.request_epoch.max(request_epoch);
                existing.source = ThumbnailRequestSource::Normal;
                existing.track_bulk_progress = false;
                existing.bulk_priority = None;
                existing.bulk_session = None;
            }

            // The sort key always changed (priority/source/index); reinsert at
            // the correct position under the pre-mutation normal state.
            let request = items.remove(existing_index);
            Self::insert_sorted(items, request, had_normal_before);

            if !was_normal {
                state.bulk_count = state.bulk_count.saturating_sub(1);
                state.normal_count += 1;
                if state.normal_count == 1 && state.bulk_count > 0 {
                    // First normal request while bulk work is queued: bulk sort
                    // keys are now demoted. Re-establish bucket ordering once.
                    Self::resort_all(&mut state);
                }
            }

            true
        };

        if promoted {
            self.condvar.notify_one();
        }

        promoted
    }

    #[allow(clippy::too_many_arguments)]
    fn push_with_index_and_source(
        &self,
        path: PathBuf,
        gen: usize,
        request_size: u32,
        priority: IOPriority,
        directory_index: Option<usize>,
        modified: u64,
        request_epoch: u64,
        source: ThumbnailRequestSource,
        bulk_session: Option<u64>,
    ) {
        let parent = path.parent().unwrap_or(&path).to_path_buf();
        let drive = Self::drive_key(&path);
        let cached_is_ssd = {
            let state = self.state.lock();
            state.drive_is_ssd.get(&drive).copied()
        };
        // Queue pushes can be triggered from UI selection/preview paths. Avoid
        // synchronous disk-profile probing here; unknown drives use HDD ordering
        // until a background path populates the shared drive profile cache.
        let detected_is_ssd = cached_is_ssd
            .or_else(|| io_priority::try_is_ssd(&path))
            .unwrap_or(false);

        {
            let mut state = self.state.lock();

            // Group by parent directory (for HDD seek optimization)
            let is_ssd = *state
                .drive_is_ssd
                .entry(drive.clone())
                .or_insert(detected_is_ssd);
            if !is_ssd && cached_is_ssd.is_none() {
                log::info!(
                    "[IO] HDD detected on drive {:?} - enabling directory grouping for seek optimization",
                    drive
                );
            }
            let request = ThumbnailRequest {
                path: path.clone(),
                generation: gen,
                size: request_size,
                request_epoch,
                priority,
                directory_index,
                modified,
                source,
                track_bulk_progress: matches!(source, ThumbnailRequestSource::BulkScan),
                bulk_priority: matches!(source, ThumbnailRequestSource::BulkScan)
                    .then_some(priority),
                bulk_session,
                queued_at: Instant::now(),
                seq: state.next_seq,
            };

            let mut needs_enqueue = true;
            if state.pending.contains(&path) {
                if Self::merge_pending_request(&mut state, &parent, &request) {
                    needs_enqueue = false;
                } else {
                    log::warn!(
                        "[THUMB-QUEUE] pending/bucket mismatch for {:?}; requeueing request",
                        path
                    );
                }
            }

            if needs_enqueue {
                state.pending.insert(path.clone());
                state.next_seq += 1;

                let was_normal = matches!(source, ThumbnailRequestSource::Normal);
                let had_normal_before = state.normal_count > 0;
                let bucket = state.by_directory.entry(parent.clone()).or_default();
                // PERF-03: O(log M) binary-search insert into the sorted
                // bucket (was: O(M log M) full sort on every HDD push).
                Self::insert_sorted(bucket, request, had_normal_before);

                if was_normal {
                    state.normal_count += 1;
                    if state.normal_count == 1 && state.bulk_count > 0 {
                        // First visible request while bulk work is queued:
                        // bulk sort keys are now demoted. Re-establish bucket
                        // ordering once for the whole interactive episode.
                        Self::resort_all(&mut state);
                    }
                } else {
                    state.bulk_count += 1;
                }
            }
        }

        self.condvar.notify_one();
    }

    fn drive_key(path: &Path) -> PathBuf {
        use std::path::Component;

        let mut components = path.components();
        match components.next() {
            Some(Component::Prefix(prefix)) => PathBuf::from(prefix.as_os_str()),
            Some(Component::RootDir) => PathBuf::from(std::path::MAIN_SEPARATOR.to_string()),
            _ => PathBuf::new(),
        }
    }

    fn is_directory_ssd(state: &QueueState, dir: &Path) -> bool {
        let drive = Self::drive_key(dir);
        state.drive_is_ssd.get(&drive).copied().unwrap_or(true)
    }

    fn merge_pending_request(
        state: &mut QueueState,
        parent: &PathBuf,
        incoming: &ThumbnailRequest,
    ) -> bool {
        if let Some(items) = state.by_directory.get_mut(parent) {
            let Some(existing_index) = items.iter().position(|req| req.path == incoming.path)
            else {
                // Defensive self-healing falls through below.
                state.pending.remove(&incoming.path);
                return false;
            };

            let was_normal = matches!(items[existing_index].source, ThumbnailRequestSource::Normal);
            let had_normal_before = state.normal_count > 0;
            let mut key_changed = false;

            {
                let existing = &mut items[existing_index];

                // Promote to the most urgent priority (Interactive < Prefetch < Background).
                if incoming.priority < existing.priority {
                    existing.priority = incoming.priority;
                    key_changed = true;
                }

                // Keep the largest requested size to avoid serving undersized thumbnails.
                if incoming.size > existing.size {
                    existing.size = incoming.size;
                }

                // Keep the newest generation so stale requests do not win.
                if incoming.generation > existing.generation {
                    existing.generation = incoming.generation;
                }

                if incoming.request_epoch > existing.request_epoch {
                    existing.request_epoch = incoming.request_epoch;
                }

                // Prefer lower directory index for earlier on-screen items.
                if let Some(new_index) = incoming.directory_index {
                    let replace_index = match existing.directory_index {
                        Some(old_index) => new_index < old_index,
                        None => true,
                    };
                    if replace_index {
                        existing.directory_index = Some(new_index);
                        key_changed = true;
                    }
                }

                // Prefer known/most recent modified timestamp when available.
                if incoming.modified > 0
                    && (existing.modified == 0 || incoming.modified > existing.modified)
                {
                    existing.modified = incoming.modified;
                }

                // If a path becomes visible to the user, treat it as a normal request
                // even if it was originally queued by the bulk scan.
                if existing.source != incoming.source
                    && matches!(incoming.source, ThumbnailRequestSource::Normal)
                {
                    existing.source = ThumbnailRequestSource::Normal;
                    key_changed = true;
                }

                if incoming.track_bulk_progress {
                    if !existing.track_bulk_progress {
                        existing.track_bulk_progress = true;
                    }

                    if existing.bulk_priority.is_none() {
                        existing.bulk_priority = incoming.bulk_priority;
                    }

                    if existing.bulk_session.is_none() {
                        existing.bulk_session = incoming.bulk_session;
                    }
                }
            }

            // PERF-03: maintain the sorted-bucket invariant incrementally.
            let became_normal =
                !was_normal && matches!(incoming.source, ThumbnailRequestSource::Normal);
            if key_changed {
                let request = items.remove(existing_index);
                Self::insert_sorted(items, request, had_normal_before);
            }
            // `items` borrow ends here; counters are updated afterwards.
            if became_normal {
                state.bulk_count = state.bulk_count.saturating_sub(1);
                state.normal_count += 1;
                if state.normal_count == 1 && state.bulk_count > 0 {
                    // Bulk→Normal transition introduced the first normal
                    // request: bulk sort keys are now demoted queue-wide.
                    Self::resort_all(state);
                }
            }

            return true;
        }

        // Defensive self-healing: pending contained path but request was missing in buckets.
        state.pending.remove(&incoming.path);
        false
    }

    /// Remove specific paths from the queue (e.g., files being deleted)
    pub fn remove_paths(&self, paths: &[PathBuf]) -> usize {
        let mut state = self.state.lock();
        let mut removed = 0usize;
        for path in paths {
            if state.pending.remove(path) {
                removed += 1;
                // Remove from the directory-grouped map
                if let Some(parent) = path.parent() {
                    let parent_buf = parent.to_path_buf();
                    if let Some(items) = state.by_directory.get_mut(&parent_buf) {
                        items.retain(|req| req.path != *path);
                        if items.is_empty() {
                            state.by_directory.remove(&parent_buf);
                        }
                    }
                }
            }
        }
        // PERF-03: removals can drop the last Normal request, restoring bulk
        // sort keys. Recount/re-sort once (rare operation).
        if removed > 0 {
            Self::recount_and_resort(&mut state);
        }
        removed
    }

    /// Cancels queued normal work and returns paths whose requests were fully
    /// removed. Requests already claimed by a worker are not returned, while
    /// merged bulk requests are restored to bulk-only work.
    pub fn cancel_normal_paths(&self, paths: &[PathBuf]) -> Vec<PathBuf> {
        let mut state = self.state.lock();
        let target_paths: FxHashSet<&PathBuf> = paths.iter().collect();
        let mut cancelled = FxHashSet::default();
        let mut retained_bulk = FxHashSet::default();

        state.by_directory.retain(|_, items| {
            items.retain_mut(|request| {
                if !target_paths.contains(&request.path) {
                    return true;
                }
                if request.track_bulk_progress {
                    request.source = ThumbnailRequestSource::BulkScan;
                    request.directory_index = None;
                    if let Some(priority) = request.bulk_priority {
                        request.priority = priority;
                    }
                    retained_bulk.insert(request.path.clone());
                    return true;
                }

                cancelled.insert(request.path.clone());
                false
            });
            !items.is_empty()
        });

        for path in paths {
            if !retained_bulk.contains(path) && state.pending.remove(path) {
                cancelled.insert(path.clone());
            }
        }
        // PERF-03: cancelled entries were removed and retained merged entries
        // had their bulk priority restored — recount and re-sort once.
        Self::recount_and_resort(&mut state);
        cancelled.into_iter().collect()
    }

    /// Cancels queued work for a bulk scan session. Pure bulk requests are
    /// removed; requests promoted to normal/visible work are kept but detached
    /// from bulk progress so current-folder thumbnails are not lost.
    pub fn cancel_bulk_scan_session(&self, session: u64) -> usize {
        let mut state = self.state.lock();
        let mut removed = 0usize;

        state.by_directory.retain(|_, items| {
            let before = items.len();
            items.retain_mut(|request| {
                if request.bulk_session != Some(session) {
                    return true;
                }

                if matches!(request.source, ThumbnailRequestSource::BulkScan) {
                    return false;
                }

                request.track_bulk_progress = false;
                request.bulk_priority = None;
                request.bulk_session = None;
                true
            });
            removed += before.saturating_sub(items.len());

            !items.is_empty()
        });

        state.pending.clear();
        let retained_paths: Vec<PathBuf> = state
            .by_directory
            .values()
            .flat_map(|items| items.iter().map(|request| request.path.clone()))
            .collect();
        state.pending.extend(retained_paths);

        if state
            .current_directory
            .as_ref()
            .is_some_and(|dir| !state.by_directory.contains_key(dir))
        {
            state.current_directory = None;
        }

        // PERF-03: retained entries may have been detached from bulk progress
        // with key changes — recount and re-sort once (rare operation).
        Self::recount_and_resort(&mut state);

        removed
    }

    /// Pop the next request, optimizing for disk locality on HDDs
    #[allow(clippy::type_complexity)]
    pub fn pop(
        &self,
    ) -> Option<(
        PathBuf,
        usize,
        u32,
        u64,
        IOPriority,
        u64,
        ThumbnailRequestSource,
        bool,
        Option<u64>,
    )> {
        let mut state = self.state.lock();

        loop {
            if state.shutdown {
                return None;
            }

            // Try to get next item
            if let Some(request) = Self::pop_next_request(&mut state) {
                state.pending.remove(&request.path);

                // PERF-03: keep the source counters in sync. When the last
                // Normal request leaves the queue, bulk sort keys are restored
                // and bucket ordering must be re-established once.
                if matches!(request.source, ThumbnailRequestSource::Normal) {
                    state.normal_count = state.normal_count.saturating_sub(1);
                    if state.normal_count == 0
                        && state.bulk_count > 0
                        && !state.by_directory.is_empty()
                    {
                        Self::resort_all(&mut state);
                    }
                } else {
                    state.bulk_count = state.bulk_count.saturating_sub(1);
                }

                log_slow_queue_wait(&request);

                // Adjust thread priority based on request priority
                io_priority::set_thread_priority(request.priority);

                return Some((
                    request.path,
                    request.generation,
                    request.size,
                    request.request_epoch,
                    request.priority,
                    request.modified,
                    request.source,
                    request.track_bulk_progress,
                    request.bulk_session,
                ));
            }

            // Wait for new work
            self.condvar.wait(&mut state);
        }
    }

    /// PERF-03: effective pop ordering key of a request. Bulk-scan requests are
    /// demoted to Background whenever any Normal request is queued so visible
    /// work always wins.
    fn item_sort_key(request: &ThumbnailRequest, has_normal: bool) -> (IOPriority, bool) {
        let is_bulk_scan = matches!(request.source, ThumbnailRequestSource::BulkScan);
        let priority = if is_bulk_scan && has_normal {
            IOPriority::Background
        } else {
            request.priority
        };

        (priority, is_bulk_scan)
    }

    /// PERF-03: full bucket ordering — effective key, then on-screen directory
    /// index, then stable enqueue order (FIFO).
    fn compare_items(
        a: &ThumbnailRequest,
        b: &ThumbnailRequest,
        has_normal: bool,
    ) -> std::cmp::Ordering {
        match Self::item_sort_key(a, has_normal).cmp(&Self::item_sort_key(b, has_normal)) {
            std::cmp::Ordering::Equal => {
                match a
                    .directory_index
                    .unwrap_or(usize::MAX)
                    .cmp(&b.directory_index.unwrap_or(usize::MAX))
                {
                    std::cmp::Ordering::Equal => a.seq.cmp(&b.seq),
                    other => other,
                }
            }
            other => other,
        }
    }

    /// PERF-03: insert into a sorted bucket via binary search (O(log M)
    /// comparisons + O(M) shift, replacing the previous O(M log M) full sort).
    fn insert_sorted(
        items: &mut Vec<ThumbnailRequest>,
        request: ThumbnailRequest,
        has_normal: bool,
    ) {
        let pos = items
            .binary_search_by(|item| Self::compare_items(item, &request, has_normal))
            .unwrap_or_else(|pos| pos);
        items.insert(pos, request);
    }

    /// PERF-03: re-sort every bucket under the current `has_normal` state.
    /// Only called on Normal-count transitions and rare batch operations.
    fn resort_all(state: &mut QueueState) {
        let has_normal = state.normal_count > 0;
        for items in state.by_directory.values_mut() {
            items.sort_by(|a, b| Self::compare_items(a, b, has_normal));
        }
    }

    /// PERF-03: rebuild the source counters from the bucket contents and
    /// re-sort. Used after rare batch operations (cancel/clear) where
    /// incremental bookkeeping would be error-prone.
    fn recount_and_resort(state: &mut QueueState) {
        let mut normal = 0usize;
        let mut bulk = 0usize;
        for items in state.by_directory.values() {
            for request in items {
                if matches!(request.source, ThumbnailRequestSource::Normal) {
                    normal += 1;
                } else {
                    bulk += 1;
                }
            }
        }
        state.normal_count = normal;
        state.bulk_count = bulk;
        Self::resort_all(state);
    }

    /// Get the next request, using locality optimization for HDDs
    fn pop_next_request(state: &mut QueueState) -> Option<ThumbnailRequest> {
        if state.by_directory.is_empty() {
            return None;
        }

        let has_normal = state.normal_count > 0;

        // Keep locality only for HDD directories.
        if let Some(current_dir) = state.current_directory.clone() {
            match state.by_directory.get(&current_dir) {
                Some(items)
                    if !items.is_empty() && !Self::is_directory_ssd(state, &current_dir) =>
                {
                    return Self::pop_with_locality(state, has_normal);
                }
                Some(_) => {}
                None => state.current_directory = None,
            }
        }

        // PERF-03: buckets are kept sorted, so each directory's best request
        // is its head — directory selection is O(D) instead of O(total items).
        let best_dir = state
            .by_directory
            .iter()
            .filter(|(_, items)| !items.is_empty())
            .min_by_key(|(_, items)| Self::item_sort_key(&items[0], has_normal))
            .map(|(dir, _)| dir.clone())?;

        if Self::is_directory_ssd(state, &best_dir) {
            state.current_directory = None;
            Self::pop_from_directory(state, &best_dir)
        } else {
            state.current_directory = Some(best_dir);
            Self::pop_with_locality(state, has_normal)
        }
    }

    /// Pop item with locality preference (HDD mode)
    fn pop_with_locality(state: &mut QueueState, has_normal: bool) -> Option<ThumbnailRequest> {
        // If we have a current directory with items, continue there
        // (unless there's a higher priority item elsewhere)
        if let Some(ref dir) = state.current_directory.clone() {
            if let Some(items) = state.by_directory.get(dir) {
                if !items.is_empty() {
                    let current_best = Self::item_sort_key(&items[0], has_normal);

                    // Preserve HDD locality for normal work, matching the old
                    // behavior: switch only for interactive requests. The extra
                    // exception is bulk-only work, which must yield to current
                    // folder requests even when priorities tie at Background.
                    let should_switch =
                        state.by_directory.iter().any(|(other_dir, other_items)| {
                            if other_dir == dir || other_items.is_empty() {
                                return false;
                            }

                            let other_best = Self::item_sort_key(&other_items[0], has_normal);
                            let interactive_switch = other_best.0 == IOPriority::Interactive
                                && current_best.0 != IOPriority::Interactive;
                            let bulk_yield_switch = current_best.1 && other_best < current_best;

                            interactive_switch || bulk_yield_switch
                        });

                    if !should_switch {
                        return Self::pop_from_directory(state, dir);
                    }
                }
            }
        }

        // Find directory with highest priority item
        let best_dir = state
            .by_directory
            .iter()
            .filter(|(_, items)| !items.is_empty())
            .min_by_key(|(_, items)| Self::item_sort_key(&items[0], has_normal))
            .map(|(dir, _)| dir.clone())?;

        state.current_directory = Some(best_dir.clone());
        Self::pop_from_directory(state, &best_dir)
    }

    /// Pop highest priority item from a specific directory
    fn pop_from_directory(state: &mut QueueState, dir: &PathBuf) -> Option<ThumbnailRequest> {
        let items = state.by_directory.get_mut(dir)?;

        if items.is_empty() {
            state.by_directory.remove(dir);
            return None;
        }

        // PERF-03: the bucket is sorted by the pop ordering, so the best
        // request is the head — O(1) selection (was: O(M) full scan per pop).
        let request = items.remove(0);

        // Clean up empty directories
        if items.is_empty() {
            state.by_directory.remove(dir);
            if state.current_directory.as_ref() == Some(dir) {
                state.current_directory = None;
            }
        }

        Some(request)
    }
}

fn log_slow_queue_wait(request: &ThumbnailRequest) {
    let queue_wait = request.queued_at.elapsed();
    if queue_wait < SLOW_QUEUE_WAIT_THRESHOLD {
        return;
    }

    let priority = match request.priority {
        IOPriority::Interactive => "interactive",
        IOPriority::Prefetch => "prefetch",
        IOPriority::Background => "background",
    };
    let source = match request.source {
        ThumbnailRequestSource::Normal => "normal",
        ThumbnailRequestSource::BulkScan => "bulk",
    };

    log::info!(
        "[THUMB-QUEUE] slow wait {:.1}ms source={} priority={} {:?}",
        queue_wait.as_millis() as f64,
        source,
        priority,
        request.path.file_name()
    );

    crate::infrastructure::diagnostic_logger::diag_info(
        "thumbnail_queue",
        "slow_wait",
        &[
            crate::infrastructure::diagnostic_logger::field_duration_ms("wait", queue_wait),
            crate::infrastructure::diagnostic_logger::field_label("source", source),
            crate::infrastructure::diagnostic_logger::field_label("priority", priority),
            crate::infrastructure::diagnostic_logger::field_bool(
                "bulk",
                matches!(request.source, ThumbnailRequestSource::BulkScan),
            ),
        ],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_read_coalescing_order_hdd() {
        let dir = tempdir().unwrap();
        let parent = dir.path().join("dir");
        std::fs::create_dir(&parent).unwrap();
        let path_a = parent.join("a.png");
        let path_b = parent.join("b.png");

        let queue = PriorityThumbnailQueue::new();
        {
            let mut state = queue.state.lock();
            state
                .drive_is_ssd
                .insert(PriorityThumbnailQueue::drive_key(&path_a), false);
        }

        queue.push_with_index(path_a.clone(), 1, 64, IOPriority::Prefetch, Some(2), 0);
        queue.push_with_index(path_b.clone(), 1, 64, IOPriority::Prefetch, Some(1), 0);

        let (path, _, _, _, _, _, _, _, _) = queue.pop().unwrap();
        assert_eq!(path, path_b);
    }

    #[test]
    fn equal_priority_non_indexed_requests_preserve_fifo_order() {
        let dir = tempdir().unwrap();
        let parent = dir.path().join("dir");
        std::fs::create_dir(&parent).unwrap();
        let path_a = parent.join("a.png");
        let path_b = parent.join("b.png");
        let path_c = parent.join("c.png");

        let queue = PriorityThumbnailQueue::new();
        {
            let mut state = queue.state.lock();
            state
                .drive_is_ssd
                .insert(PriorityThumbnailQueue::drive_key(&path_a), true);
        }

        queue.push(path_a.clone(), 1, 64, IOPriority::Prefetch, 0);
        queue.push(path_b.clone(), 1, 64, IOPriority::Prefetch, 0);
        queue.push(path_c.clone(), 1, 64, IOPriority::Prefetch, 0);

        let (path, _, _, _, _, _, _, _, _) = queue.pop().unwrap();
        assert_eq!(path, path_a);
        let (path, _, _, _, _, _, _, _, _) = queue.pop().unwrap();
        assert_eq!(path, path_b);
        let (path, _, _, _, _, _, _, _, _) = queue.pop().unwrap();
        assert_eq!(path, path_c);
    }

    #[test]
    fn cancelling_normal_paths_does_not_report_work_claimed_by_a_worker() {
        let dir = tempdir().unwrap();
        let in_flight = dir.path().join("in-flight.png");
        let queued = dir.path().join("queued.png");
        let queue = PriorityThumbnailQueue::new();
        queue.push(in_flight.clone(), 1, 64, IOPriority::Prefetch, 0);
        queue.push(queued.clone(), 1, 64, IOPriority::Prefetch, 0);

        let (claimed, _, _, _, _, _, _, _, _) = queue.pop().unwrap();
        assert_eq!(claimed, in_flight);

        assert_eq!(
            queue.cancel_normal_paths(&[in_flight, queued.clone()]),
            vec![queued]
        );
    }

    #[test]
    fn cancelling_normal_path_preserves_merged_bulk_work() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("merged.png");
        let queue = PriorityThumbnailQueue::new();
        queue.push_bulk_scan(path.clone(), 1, 256, IOPriority::Background, 0, 7);
        queue.push(path.clone(), 1, 64, IOPriority::Interactive, 0);

        assert!(queue
            .cancel_normal_paths(std::slice::from_ref(&path))
            .is_empty());
        let (popped, _, _, _, priority, _, source, track_bulk_progress, bulk_session) =
            queue.pop().unwrap();
        assert_eq!(popped, path);
        assert_eq!(priority, IOPriority::Background);
        assert_eq!(source, ThumbnailRequestSource::BulkScan);
        assert!(track_bulk_progress);
        assert_eq!(bulk_session, Some(7));
    }

    #[test]
    fn test_deduplication() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.jpg");

        let queue = PriorityThumbnailQueue::new();

        // Push same path twice
        queue.push_with_index(path.clone(), 1, 64, IOPriority::Background, Some(10), 0);
        queue.push_with_index(path.clone(), 2, 256, IOPriority::Interactive, Some(2), 123);

        // Should only get one back, with merged/upgraded fields
        let result = queue.pop();
        assert!(result.is_some());
        let (p, g, size, _, priority, modified, source, _, _) = result.unwrap();
        assert_eq!(p, path);
        assert_eq!(g, 2);
        assert_eq!(size, 256);
        assert_eq!(priority, IOPriority::Interactive);
        assert_eq!(modified, 123);
        assert_eq!(source, ThumbnailRequestSource::Normal);
    }

    #[test]
    fn promote_pending_to_interactive_moves_selected_request_first() {
        let dir = tempdir().unwrap();
        let parent = dir.path().join("dir");
        std::fs::create_dir(&parent).unwrap();
        let earlier_path = parent.join("a.jpg");
        let selected_path = parent.join("z.jpg");

        let queue = PriorityThumbnailQueue::new();
        {
            let mut state = queue.state.lock();
            state
                .drive_is_ssd
                .insert(PriorityThumbnailQueue::drive_key(&selected_path), true);
        }

        queue.push_with_index(
            earlier_path.clone(),
            1,
            128,
            IOPriority::Interactive,
            Some(1),
            0,
        );
        queue.push_with_index(
            selected_path.clone(),
            1,
            64,
            IOPriority::Prefetch,
            Some(50),
            0,
        );

        assert!(queue.promote_pending_to_interactive(&selected_path, 2, 512, 0, 123, 0));

        let (path, gen, size, _, priority, modified, source, track_bulk_progress, bulk_session) =
            queue.pop().unwrap();
        assert_eq!(path, selected_path);
        assert_eq!(gen, 2);
        assert_eq!(size, 512);
        assert_eq!(priority, IOPriority::Interactive);
        assert_eq!(modified, 123);
        assert_eq!(source, ThumbnailRequestSource::Normal);
        assert!(!track_bulk_progress);
        assert_eq!(bulk_session, None);
    }

    #[test]
    fn test_requeue_when_pending_bucket_mismatch() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("mismatch.jpg");

        let queue = PriorityThumbnailQueue::new();
        {
            let mut state = queue.state.lock();
            state.pending.insert(path.clone());
        }

        queue.push_with_index(path.clone(), 3, 128, IOPriority::Interactive, Some(5), 321);

        let result = queue.pop();
        assert!(result.is_some());
        let (p, g, size, _, priority, modified, source, _, _) = result.unwrap();
        assert_eq!(p, path);
        assert_eq!(g, 3);
        assert_eq!(size, 128);
        assert_eq!(priority, IOPriority::Interactive);
        assert_eq!(modified, 321);
        assert_eq!(source, ThumbnailRequestSource::Normal);
    }

    #[test]
    fn clear_pending_preserves_bulk_scan_work() {
        let dir = tempdir().unwrap();
        let parent = dir.path().join("dir");
        std::fs::create_dir(&parent).unwrap();
        let normal_path = parent.join("visible.jpg");
        let bulk_path = parent.join("bulk.jpg");

        let queue = PriorityThumbnailQueue::new();
        {
            let mut state = queue.state.lock();
            state
                .drive_is_ssd
                .insert(PriorityThumbnailQueue::drive_key(&normal_path), true);
        }

        queue.push(normal_path, 1, 128, IOPriority::Interactive, 0);
        queue.push_bulk_scan(bulk_path.clone(), 1, 512, IOPriority::Prefetch, 0, 1);

        assert_eq!(queue.clear_pending(), 1);
        assert_eq!(queue.pending_count(), 1);

        let (path, _, size, _, priority, _, source, track_bulk_progress, bulk_session) =
            queue.pop().unwrap();
        assert_eq!(path, bulk_path);
        assert_eq!(size, 512);
        assert_eq!(priority, IOPriority::Prefetch);
        assert_eq!(source, ThumbnailRequestSource::BulkScan);
        assert!(track_bulk_progress);
        assert_eq!(bulk_session, Some(1));
    }

    #[test]
    fn clear_pending_restores_promoted_bulk_scan_priority() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("promoted.jpg");

        let queue = PriorityThumbnailQueue::new();
        {
            let mut state = queue.state.lock();
            state
                .drive_is_ssd
                .insert(PriorityThumbnailQueue::drive_key(&path), true);
        }

        queue.push_bulk_scan(path.clone(), 1, 512, IOPriority::Prefetch, 0, 7);
        queue.push_with_index(path.clone(), 2, 512, IOPriority::Interactive, Some(0), 123);

        assert_eq!(queue.clear_pending(), 0);
        assert_eq!(queue.pending_count(), 1);

        let (popped_path, _, _, _, priority, modified, source, track_bulk_progress, bulk_session) =
            queue.pop().unwrap();
        assert_eq!(popped_path, path);
        assert_eq!(priority, IOPriority::Prefetch);
        assert_eq!(modified, 123);
        assert_eq!(source, ThumbnailRequestSource::BulkScan);
        assert!(track_bulk_progress);
        assert_eq!(bulk_session, Some(7));
    }

    #[test]
    fn bulk_scan_waits_behind_normal_requests() {
        let dir = tempdir().unwrap();
        let parent = dir.path().join("dir");
        std::fs::create_dir(&parent).unwrap();
        let bulk_path = parent.join("bulk.jpg");
        let normal_path = parent.join("normal.jpg");

        let queue = PriorityThumbnailQueue::new();
        {
            let mut state = queue.state.lock();
            state
                .drive_is_ssd
                .insert(PriorityThumbnailQueue::drive_key(&bulk_path), true);
        }

        queue.push_bulk_scan(bulk_path.clone(), 1, 512, IOPriority::Prefetch, 0, 1);
        queue.push(normal_path.clone(), 2, 128, IOPriority::Prefetch, 0);

        let (path, _, _, _, _, _, source, track_bulk_progress, _) = queue.pop().unwrap();
        assert_eq!(path, normal_path);
        assert_eq!(source, ThumbnailRequestSource::Normal);
        assert!(!track_bulk_progress);

        let (path, _, _, _, priority, _, source, track_bulk_progress, bulk_session) =
            queue.pop().unwrap();
        assert_eq!(path, bulk_path);
        assert_eq!(priority, IOPriority::Prefetch);
        assert_eq!(source, ThumbnailRequestSource::BulkScan);
        assert!(track_bulk_progress);
        assert_eq!(bulk_session, Some(1));
    }

    #[test]
    fn cancel_bulk_scan_session_removes_only_matching_bulk_requests() {
        let dir = tempdir().unwrap();
        let parent = dir.path().join("dir");
        std::fs::create_dir(&parent).unwrap();
        let cancelled_bulk = parent.join("cancelled.jpg");
        let active_bulk = parent.join("active.jpg");
        let normal_path = parent.join("normal.jpg");

        let queue = PriorityThumbnailQueue::new();
        {
            let mut state = queue.state.lock();
            state
                .drive_is_ssd
                .insert(PriorityThumbnailQueue::drive_key(&cancelled_bulk), true);
        }

        queue.push_bulk_scan(cancelled_bulk, 1, 512, IOPriority::Prefetch, 0, 1);
        queue.push_bulk_scan(active_bulk.clone(), 1, 512, IOPriority::Prefetch, 0, 2);
        queue.push(normal_path.clone(), 2, 128, IOPriority::Prefetch, 0);

        assert_eq!(queue.cancel_bulk_scan_session(1), 1);
        assert_eq!(queue.pending_count(), 2);

        let (path, _, _, _, _, _, source, track_bulk_progress, bulk_session) = queue.pop().unwrap();
        assert_eq!(path, normal_path);
        assert_eq!(source, ThumbnailRequestSource::Normal);
        assert!(!track_bulk_progress);
        assert_eq!(bulk_session, None);

        let (path, _, _, _, _, _, source, track_bulk_progress, bulk_session) = queue.pop().unwrap();
        assert_eq!(path, active_bulk);
        assert_eq!(source, ThumbnailRequestSource::BulkScan);
        assert!(track_bulk_progress);
        assert_eq!(bulk_session, Some(2));
    }

    #[test]
    fn cancel_bulk_scan_session_detaches_promoted_normal_request() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("promoted-visible.jpg");

        let queue = PriorityThumbnailQueue::new();
        {
            let mut state = queue.state.lock();
            state
                .drive_is_ssd
                .insert(PriorityThumbnailQueue::drive_key(&path), true);
        }

        queue.push_bulk_scan(path.clone(), 1, 512, IOPriority::Prefetch, 0, 3);
        queue.push_with_index(path.clone(), 2, 512, IOPriority::Interactive, Some(0), 123);

        assert_eq!(queue.cancel_bulk_scan_session(3), 0);
        assert_eq!(queue.pending_count(), 1);

        let (popped_path, _, _, _, priority, modified, source, track_bulk_progress, bulk_session) =
            queue.pop().unwrap();
        assert_eq!(popped_path, path);
        assert_eq!(priority, IOPriority::Interactive);
        assert_eq!(modified, 123);
        assert_eq!(source, ThumbnailRequestSource::Normal);
        assert!(!track_bulk_progress);
        assert_eq!(bulk_session, None);
    }

    #[test]
    fn hdd_locality_yields_bulk_scan_to_normal_requests() {
        let dir = tempdir().unwrap();
        let bulk_dir = dir.path().join("bulk");
        let normal_dir = dir.path().join("normal");
        std::fs::create_dir(&bulk_dir).unwrap();
        std::fs::create_dir(&normal_dir).unwrap();
        let bulk_a = bulk_dir.join("a.jpg");
        let bulk_b = bulk_dir.join("b.jpg");
        let normal_path = normal_dir.join("normal.jpg");

        let queue = PriorityThumbnailQueue::new();
        {
            let mut state = queue.state.lock();
            state
                .drive_is_ssd
                .insert(PriorityThumbnailQueue::drive_key(&bulk_a), false);
        }

        queue.push_bulk_scan(bulk_a.clone(), 1, 512, IOPriority::Prefetch, 0, 1);
        queue.push_bulk_scan(bulk_b.clone(), 1, 512, IOPriority::Prefetch, 0, 1);

        let (first_path, _, _, _, _, _, first_source, _, first_session) = queue.pop().unwrap();
        assert!(first_path == bulk_a || first_path == bulk_b);
        assert_eq!(first_source, ThumbnailRequestSource::BulkScan);
        assert_eq!(first_session, Some(1));

        queue.push(normal_path.clone(), 2, 128, IOPriority::Background, 0);

        let (path, _, _, _, _, _, source, track_bulk_progress, _) = queue.pop().unwrap();
        assert_eq!(path, normal_path);
        assert_eq!(source, ThumbnailRequestSource::Normal);
        assert!(!track_bulk_progress);

        let (path, _, _, _, priority, _, source, track_bulk_progress, bulk_session) =
            queue.pop().unwrap();
        assert!(path == bulk_a || path == bulk_b);
        assert_eq!(priority, IOPriority::Prefetch);
        assert_eq!(source, ThumbnailRequestSource::BulkScan);
        assert!(track_bulk_progress);
        assert_eq!(bulk_session, Some(1));
    }

    /// PERF-03 regression guard: when the last Normal request leaves the
    /// queue the bulk sort keys are restored and the buckets are re-sorted
    /// once; remaining bulk work must keep FIFO enqueue order.
    #[test]
    fn bulk_fifo_order_restored_after_last_normal_pops() {
        let dir = tempdir().unwrap();
        let parent = dir.path().join("dir");
        std::fs::create_dir(&parent).unwrap();
        let bulk_a = parent.join("bulk-a.jpg");
        let bulk_b = parent.join("bulk-b.jpg");
        let normal_path = parent.join("normal.jpg");

        let queue = PriorityThumbnailQueue::new();
        {
            let mut state = queue.state.lock();
            state
                .drive_is_ssd
                .insert(PriorityThumbnailQueue::drive_key(&bulk_a), false);
        }

        queue.push_bulk_scan(bulk_a.clone(), 1, 512, IOPriority::Prefetch, 0, 1);
        queue.push_bulk_scan(bulk_b.clone(), 1, 512, IOPriority::Prefetch, 0, 1);
        queue.push(normal_path.clone(), 2, 128, IOPriority::Interactive, 0);

        let (path, _, _, _, _, _, source, _, _) = queue.pop().unwrap();
        assert_eq!(path, normal_path);
        assert_eq!(source, ThumbnailRequestSource::Normal);

        // After the normal→0 transition the bucket was re-sorted under
        // restored keys; FIFO enqueue order must hold.
        let (path, _, _, _, _, _, source, _, _) = queue.pop().unwrap();
        assert_eq!(path, bulk_a);
        assert_eq!(source, ThumbnailRequestSource::BulkScan);

        let (path, _, _, _, _, _, source, _, _) = queue.pop().unwrap();
        assert_eq!(path, bulk_b);
        assert_eq!(source, ThumbnailRequestSource::BulkScan);
    }

    /// PERF-03 regression guard: an interactive request pushed into a bucket
    /// that already holds ordered bulk work must jump to the head without
    /// disturbing the directory-index ordering of the remaining bulk items.
    #[test]
    fn interactive_insert_into_ordered_bulk_bucket_keeps_index_ordering() {
        let dir = tempdir().unwrap();
        let parent = dir.path().join("dir");
        std::fs::create_dir(&parent).unwrap();
        let bulk_late = parent.join("bulk-late.jpg");
        let bulk_early = parent.join("bulk-early.jpg");
        let selected = parent.join("selected.jpg");

        let queue = PriorityThumbnailQueue::new();
        {
            let mut state = queue.state.lock();
            state
                .drive_is_ssd
                .insert(PriorityThumbnailQueue::drive_key(&bulk_late), false);
        }

        queue.push_with_index(bulk_late.clone(), 1, 512, IOPriority::Prefetch, Some(9), 0);
        queue.push_with_index(bulk_early.clone(), 1, 512, IOPriority::Prefetch, Some(2), 0);
        queue.push_with_index(
            selected.clone(),
            2,
            512,
            IOPriority::Interactive,
            Some(5),
            0,
        );

        let (path, _, _, _, priority, _, _, _, _) = queue.pop().unwrap();
        assert_eq!(path, selected);
        assert_eq!(priority, IOPriority::Interactive);

        let (path, _, _, _, _, _, _, _, _) = queue.pop().unwrap();
        assert_eq!(path, bulk_early);

        let (path, _, _, _, _, _, _, _, _) = queue.pop().unwrap();
        assert_eq!(path, bulk_late);
    }
}
