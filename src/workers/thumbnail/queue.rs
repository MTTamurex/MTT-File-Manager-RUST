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
    /// Requests grouped by parent directory (for HDD locality optimization)
    by_directory: FxHashMap<PathBuf, Vec<ThumbnailRequest>>,

    /// Quick lookup to prevent duplicates
    pending: FxHashSet<PathBuf>,

    /// Per-drive storage class cache (true = SSD, false = HDD)
    drive_is_ssd: FxHashMap<PathBuf, bool>,

    /// Current directory being processed (for HDD locality)
    current_directory: Option<PathBuf>,

    /// Shutdown flag
    shutdown: bool,
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
            let is_ssd = Self::is_directory_ssd(&state, &parent);

            let Some(items) = state.by_directory.get_mut(&parent) else {
                return false;
            };

            let Some(existing) = items.iter_mut().find(|req| req.path.as_path() == path) else {
                return false;
            };

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

            if !is_ssd {
                items.sort_by(|a, b| match a.priority.cmp(&b.priority) {
                    std::cmp::Ordering::Equal => a.directory_index.cmp(&b.directory_index),
                    other => other,
                });
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
            };

            let mut needs_enqueue = true;
            if state.pending.contains(&path) {
                if Self::merge_pending_request(&mut state, &parent, &request, is_ssd) {
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

                state
                    .by_directory
                    .entry(parent.clone())
                    .or_default()
                    .push(request);

                if !is_ssd {
                    if let Some(items) = state.by_directory.get_mut(&parent) {
                        items.sort_by(|a, b| match a.priority.cmp(&b.priority) {
                            std::cmp::Ordering::Equal => a.directory_index.cmp(&b.directory_index),
                            other => other,
                        });
                    }
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
        is_ssd: bool,
    ) -> bool {
        if let Some(items) = state.by_directory.get_mut(parent) {
            if let Some(existing) = items.iter_mut().find(|req| req.path == incoming.path) {
                let mut updated = false;

                // Promote to the most urgent priority (Interactive < Prefetch < Background).
                if incoming.priority < existing.priority {
                    existing.priority = incoming.priority;
                    updated = true;
                }

                // Keep the largest requested size to avoid serving undersized thumbnails.
                if incoming.size > existing.size {
                    existing.size = incoming.size;
                    updated = true;
                }

                // Keep the newest generation so stale requests do not win.
                if incoming.generation > existing.generation {
                    existing.generation = incoming.generation;
                    updated = true;
                }

                if incoming.request_epoch > existing.request_epoch {
                    existing.request_epoch = incoming.request_epoch;
                    updated = true;
                }

                // Prefer lower directory index for earlier on-screen items.
                if let Some(new_index) = incoming.directory_index {
                    let replace_index = match existing.directory_index {
                        Some(old_index) => new_index < old_index,
                        None => true,
                    };
                    if replace_index {
                        existing.directory_index = Some(new_index);
                        updated = true;
                    }
                }

                // Prefer known/most recent modified timestamp when available.
                if incoming.modified > 0
                    && (existing.modified == 0 || incoming.modified > existing.modified)
                {
                    existing.modified = incoming.modified;
                    updated = true;
                }

                // If a path becomes visible to the user, treat it as a normal request
                // even if it was originally queued by the bulk scan.
                if existing.source != incoming.source
                    && matches!(incoming.source, ThumbnailRequestSource::Normal)
                {
                    existing.source = ThumbnailRequestSource::Normal;
                    updated = true;
                }

                if incoming.track_bulk_progress {
                    if !existing.track_bulk_progress {
                        existing.track_bulk_progress = true;
                        updated = true;
                    }

                    if existing.bulk_priority.is_none() {
                        existing.bulk_priority = incoming.bulk_priority;
                        updated = true;
                    }

                    if existing.bulk_session.is_none() {
                        existing.bulk_session = incoming.bulk_session;
                        updated = true;
                    }
                }

                if updated && !is_ssd {
                    items.sort_by(|a, b| match a.priority.cmp(&b.priority) {
                        std::cmp::Ordering::Equal => a.directory_index.cmp(&b.directory_index),
                        other => other,
                    });
                }

                return true;
            }
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
        removed
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

    /// Get the next request, using locality optimization for HDDs
    fn pop_next_request(state: &mut QueueState) -> Option<ThumbnailRequest> {
        if state.by_directory.is_empty() {
            return None;
        }

        // Keep locality only for HDD directories.
        if let Some(current_dir) = state.current_directory.clone() {
            match state.by_directory.get(&current_dir) {
                Some(items)
                    if !items.is_empty() && !Self::is_directory_ssd(state, &current_dir) =>
                {
                    return Self::pop_with_locality(state);
                }
                Some(_) => {}
                None => state.current_directory = None,
            }
        }

        let has_normal_requests = Self::has_normal_requests(state);

        // Find directory with highest-priority pending item.
        let best_dir = state
            .by_directory
            .iter()
            .filter(|(_, items)| !items.is_empty())
            .min_by_key(|(_, items)| Self::best_priority_key(items, has_normal_requests))
            .map(|(dir, _)| dir.clone())?;

        if Self::is_directory_ssd(state, &best_dir) {
            state.current_directory = None;
            Self::pop_from_directory(state, &best_dir, has_normal_requests)
        } else {
            state.current_directory = Some(best_dir);
            Self::pop_with_locality(state)
        }
    }

    /// Pop item with locality preference (HDD mode)
    fn pop_with_locality(state: &mut QueueState) -> Option<ThumbnailRequest> {
        let has_normal_requests = Self::has_normal_requests(state);

        // If we have a current directory with items, continue there
        // (unless there's a higher priority item elsewhere)
        if let Some(ref dir) = state.current_directory.clone() {
            if let Some(items) = state.by_directory.get(dir) {
                if !items.is_empty() {
                    let current_best = Self::best_priority_key(items, has_normal_requests);

                    // Preserve HDD locality for normal work, matching the old
                    // behavior: switch only for interactive requests. The extra
                    // exception is bulk-only work, which must yield to current
                    // folder requests even when priorities tie at Background.
                    let should_switch =
                        state.by_directory.iter().any(|(other_dir, other_items)| {
                            if other_dir == dir || other_items.is_empty() {
                                return false;
                            }

                            let other_best =
                                Self::best_priority_key(other_items, has_normal_requests);
                            let interactive_switch = other_best.0 == IOPriority::Interactive
                                && current_best.0 != IOPriority::Interactive;
                            let bulk_yield_switch = current_best.1 && other_best < current_best;

                            interactive_switch || bulk_yield_switch
                        });

                    if !should_switch {
                        return Self::pop_from_directory(state, dir, has_normal_requests);
                    }
                }
            }
        }

        // Find directory with highest priority item
        let best_dir = state
            .by_directory
            .iter()
            .filter(|(_, items)| !items.is_empty())
            .min_by_key(|(_, items)| Self::best_priority_key(items, has_normal_requests))
            .map(|(dir, _)| dir.clone())?;

        state.current_directory = Some(best_dir.clone());
        Self::pop_from_directory(state, &best_dir, has_normal_requests)
    }

    fn has_normal_requests(state: &QueueState) -> bool {
        state.by_directory.values().any(|items| {
            items
                .iter()
                .any(|request| matches!(request.source, ThumbnailRequestSource::Normal))
        })
    }

    fn best_priority_key(
        items: &[ThumbnailRequest],
        has_normal_requests: bool,
    ) -> (IOPriority, bool) {
        items
            .iter()
            .map(|request| Self::priority_key(request, has_normal_requests))
            .min()
            .unwrap_or((IOPriority::Background, true))
    }

    fn priority_key(request: &ThumbnailRequest, has_normal_requests: bool) -> (IOPriority, bool) {
        let is_bulk_scan = matches!(request.source, ThumbnailRequestSource::BulkScan);
        let priority = if is_bulk_scan && has_normal_requests {
            IOPriority::Background
        } else {
            request.priority
        };

        (priority, is_bulk_scan)
    }

    /// Pop highest priority item from a specific directory
    fn pop_from_directory(
        state: &mut QueueState,
        dir: &PathBuf,
        has_normal_requests: bool,
    ) -> Option<ThumbnailRequest> {
        let items = state.by_directory.get_mut(dir)?;

        if items.is_empty() {
            state.by_directory.remove(dir);
            return None;
        }

        let best_idx = items
            .iter()
            .enumerate()
            .min_by(|(idx_a, a), (idx_b, b)| {
                match Self::priority_key(a, has_normal_requests)
                    .cmp(&Self::priority_key(b, has_normal_requests))
                {
                    std::cmp::Ordering::Equal => {
                        let a_index = a.directory_index.unwrap_or(usize::MAX);
                        let b_index = b.directory_index.unwrap_or(usize::MAX);
                        match a_index.cmp(&b_index) {
                            // Preserve request order for equally-prioritized,
                            // non-indexed work. Using LIFO here made the grid's
                            // leftmost visible cells load last on SSD because the
                            // renderer queues requests left-to-right.
                            std::cmp::Ordering::Equal => idx_a.cmp(idx_b),
                            other => other,
                        }
                    }
                    other => other,
                }
            })
            .map(|(idx, _)| idx)?;

        let request = items.remove(best_idx);

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
}
