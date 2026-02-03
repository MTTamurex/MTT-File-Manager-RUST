//! Priority thumbnail queue with HDD/SSD optimization
//!
//! Groups requests by directory on HDDs to minimize seek times.

use crate::infrastructure::io_priority::{self, IOPriority};
use crate::workers::thumbnail::types::ThumbnailRequest;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::PathBuf;
use std::sync::{Condvar, Mutex};

/// Queue state with directory-grouped requests for HDD optimization
struct QueueState {
    /// Requests grouped by parent directory (for HDD locality optimization)
    by_directory: FxHashMap<PathBuf, Vec<ThumbnailRequest>>,

    /// Quick lookup to prevent duplicates
    pending: FxHashSet<PathBuf>,

    /// Whether we're on an SSD (detected on first request)
    is_ssd: Option<bool>,

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
                is_ssd: None,
                current_directory: None,
                shutdown: false,
            }),
            condvar: Condvar::new(),
        }
    }

    pub fn shutdown(&self) {
        let mut state = self.state.lock().unwrap();
        state.shutdown = true;
        self.condvar.notify_all();
    }

    /// Push a thumbnail request with the new IOPriority system
    pub fn push(&self, path: PathBuf, gen: usize, request_size: u32, priority: IOPriority, modified: u64) {
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
        let mut state = self.state.lock().unwrap();

        // Deduplication: if already pending, skip
        if !state.pending.insert(path.clone()) {
            return;
        }

        // Detect disk type on first request
        if state.is_ssd.is_none() {
            state.is_ssd = Some(io_priority::is_ssd(&path));
            if !state.is_ssd.unwrap() {
                eprintln!("[IO] HDD detected - enabling directory grouping for seek optimization");
            }
        }

        // Group by parent directory (for HDD seek optimization)
        let parent = path.parent().unwrap_or(&path).to_path_buf();

        let request = ThumbnailRequest {
            path,
            generation: gen,
            size: request_size,
            priority,
            directory_index,
            modified,
        };

        state.by_directory.entry(parent.clone()).or_default().push(request);

        if !state.is_ssd.unwrap_or(true) {
            if let Some(items) = state.by_directory.get_mut(&parent) {
                items.sort_by(|a, b| match a.priority.cmp(&b.priority) {
                    std::cmp::Ordering::Equal => a.directory_index.cmp(&b.directory_index),
                    other => other,
                });
            }
        }

        self.condvar.notify_one();
    }

    /// Pop the next request, optimizing for disk locality on HDDs
    pub fn pop(&self) -> Option<(PathBuf, usize, u32, IOPriority, u64)> {
        let mut state = self.state.lock().unwrap();

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
            state = self.condvar.wait(state).unwrap();
        }
    }

    /// Get the next request, using locality optimization for HDDs
    fn pop_next_request(state: &mut QueueState) -> Option<ThumbnailRequest> {
        if state.by_directory.is_empty() {
            return None;
        }

        let is_ssd = state.is_ssd.unwrap_or(true);

        if is_ssd {
            // SSD: Just get highest priority item from any directory
            Self::pop_highest_priority(state)
        } else {
            // HDD: Prefer items from current directory to minimize seeks
            Self::pop_with_locality(state)
        }
    }

    /// Pop highest priority item regardless of directory (SSD mode)
    fn pop_highest_priority(state: &mut QueueState) -> Option<ThumbnailRequest> {
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

        Self::pop_from_directory(state, &best_dir)
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
        let items = state.by_directory.get_mut(dir)?;

        if items.is_empty() {
            state.by_directory.remove(dir);
            return None;
        }

        let is_ssd = state.is_ssd.unwrap_or(true);
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
            state.is_ssd = Some(false);
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
        queue.push(path.clone(), 1, 64, IOPriority::Interactive, 0);
        queue.push(path.clone(), 1, 64, IOPriority::Background, 0);
        
        // Should only get one back
        let result = queue.pop();
        assert!(result.is_some());
        
        // Second pop should be None (queue is empty)
        // Note: This would block, so we can't test it directly without a timeout
    }
}