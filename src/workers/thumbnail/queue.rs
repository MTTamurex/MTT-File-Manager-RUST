//! Priority thumbnail queue with HDD/SSD optimization
//!
//! Groups requests by directory on HDDs to minimize seek times.

use crate::infrastructure::io_priority::{self, IOPriority};
use crate::workers::thumbnail::types::ThumbnailRequest;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};

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
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.shutdown = true;
        self.condvar.notify_all();
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
        self.push_with_index(path, gen, request_size, priority, None, modified);
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
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());

        // Group by parent directory (for HDD seek optimization)
        let parent = path.parent().unwrap_or(&path).to_path_buf();
        let is_ssd = Self::detect_drive_class(&mut state, &path);
        let request = ThumbnailRequest {
            path: path.clone(),
            generation: gen,
            size: request_size,
            priority,
            directory_index,
            modified,
        };

        // Deduplication with merge: upgrade existing request instead of dropping.
        if state.pending.contains(&path) {
            if Self::merge_pending_request(&mut state, &parent, &request, is_ssd) {
                self.condvar.notify_one();
            }
            return;
        }

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

    fn detect_drive_class(state: &mut QueueState, path: &Path) -> bool {
        let drive = Self::drive_key(path);
        if let Some(is_ssd) = state.drive_is_ssd.get(&drive) {
            return *is_ssd;
        }

        let is_ssd = io_priority::is_ssd(path);
        state.drive_is_ssd.insert(drive.clone(), is_ssd);
        if !is_ssd {
            log::info!(
                "[IO] HDD detected on drive {:?} - enabling directory grouping for seek optimization",
                drive
            );
        }
        is_ssd
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

                if updated && !is_ssd {
                    items.sort_by(|a, b| match a.priority.cmp(&b.priority) {
                        std::cmp::Ordering::Equal => a.directory_index.cmp(&b.directory_index),
                        other => other,
                    });
                }

                return updated;
            }
        }

        // Defensive self-healing: pending contained path but request was missing in buckets.
        state.pending.remove(&incoming.path);
        false
    }

    /// Remove specific paths from the queue (e.g., files being deleted)
    pub fn remove_paths(&self, paths: &[PathBuf]) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        for path in paths {
            if state.pending.remove(path) {
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
    }

    /// Pop the next request, optimizing for disk locality on HDDs
    pub fn pop(&self) -> Option<(PathBuf, usize, u32, IOPriority, u64)> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());

        loop {
            if state.shutdown {
                return None;
            }

            // Try to get next item
            if let Some(request) = Self::pop_next_request(&mut state) {
                state.pending.remove(&request.path);

                // Adjust thread priority based on request priority
                io_priority::set_thread_priority(request.priority);

                return Some((
                    request.path,
                    request.generation,
                    request.size,
                    request.priority,
                    request.modified,
                ));
            }

            // Wait for new work
            state = self.condvar.wait(state).unwrap_or_else(|e| e.into_inner());
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

        // Find directory with highest-priority pending item.
        let best_dir = state
            .by_directory
            .iter()
            .filter(|(_, items)| !items.is_empty())
            .min_by_key(|(_, items)| {
                items
                    .iter()
                    .map(|r| r.priority)
                    .min()
                    .unwrap_or(IOPriority::Background)
            })
            .map(|(dir, _)| dir.clone())?;

        if Self::is_directory_ssd(state, &best_dir) {
            state.current_directory = None;
            Self::pop_from_directory(state, &best_dir)
        } else {
            state.current_directory = Some(best_dir);
            Self::pop_with_locality(state)
        }
    }

    /// Pop item with locality preference (HDD mode)
    fn pop_with_locality(state: &mut QueueState) -> Option<ThumbnailRequest> {
        // If we have a current directory with items, continue there
        // (unless there's a higher priority item elsewhere)
        if let Some(ref dir) = state.current_directory.clone() {
            if let Some(items) = state.by_directory.get(dir) {
                if !items.is_empty() {
                    // Check if current dir has interactive priority
                    let current_best = items
                        .iter()
                        .map(|r| r.priority)
                        .min()
                        .unwrap_or(IOPriority::Background);

                    // Only switch directories if there's an Interactive request elsewhere
                    let should_switch =
                        state.by_directory.iter().any(|(other_dir, other_items)| {
                            other_dir != dir
                                && other_items
                                    .iter()
                                    .any(|r| r.priority == IOPriority::Interactive)
                                && current_best != IOPriority::Interactive
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
            .min_by_key(|(_, items)| {
                items
                    .iter()
                    .map(|r| r.priority)
                    .min()
                    .unwrap_or(IOPriority::Background)
            })
            .map(|(dir, _)| dir.clone())?;

        state.current_directory = Some(best_dir.clone());
        Self::pop_from_directory(state, &best_dir)
    }

    /// Pop highest priority item from a specific directory
    fn pop_from_directory(state: &mut QueueState, dir: &PathBuf) -> Option<ThumbnailRequest> {
        let is_ssd = Self::is_directory_ssd(state, dir);
        let items = state.by_directory.get_mut(dir)?;

        if items.is_empty() {
            state.by_directory.remove(dir);
            return None;
        }

        let best_idx = if is_ssd {
            items
                .iter()
                .enumerate()
                .min_by(|(idx_a, a), (idx_b, b)| match a.priority.cmp(&b.priority) {
                    std::cmp::Ordering::Equal => idx_b.cmp(idx_a),
                    other => other,
                })
                .map(|(idx, _)| idx)?
        } else {
            items
                .iter()
                .enumerate()
                .min_by(|(idx_a, a), (idx_b, b)| match a.priority.cmp(&b.priority) {
                    std::cmp::Ordering::Equal => {
                        let a_index = a.directory_index.unwrap_or(usize::MAX);
                        let b_index = b.directory_index.unwrap_or(usize::MAX);
                        match a_index.cmp(&b_index) {
                            std::cmp::Ordering::Equal => idx_b.cmp(idx_a),
                            other => other,
                        }
                    }
                    other => other,
                })
                .map(|(idx, _)| idx)?
        };

        let request = items.swap_remove(best_idx);

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
            let mut state = queue.state.lock().unwrap();
            state
                .drive_is_ssd
                .insert(PriorityThumbnailQueue::drive_key(&path_a), false);
        }

        queue.push_with_index(path_a.clone(), 1, 64, IOPriority::Prefetch, Some(2), 0);
        queue.push_with_index(path_b.clone(), 1, 64, IOPriority::Prefetch, Some(1), 0);

        let (path, _, _, _, _) = queue.pop().unwrap();
        assert_eq!(path, path_b);
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
        let (p, g, size, priority, modified) = result.unwrap();
        assert_eq!(p, path);
        assert_eq!(g, 2);
        assert_eq!(size, 256);
        assert_eq!(priority, IOPriority::Interactive);
        assert_eq!(modified, 123);
    }
}
