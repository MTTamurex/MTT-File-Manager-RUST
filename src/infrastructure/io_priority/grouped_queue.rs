use std::path::{Path, PathBuf};

use rustc_hash::FxHashMap;

use super::{is_ssd, IOPriority};

/// Groups requests by directory to minimize disk seeks on HDDs.
pub struct DirectoryGroupedQueue<T> {
    /// Items grouped by parent directory.
    by_directory: FxHashMap<PathBuf, Vec<(IOPriority, T)>>,
    /// Whether we're on an SSD (skip grouping optimization).
    is_ssd: bool,
    /// Current directory being processed (for HDD locality optimization).
    current_directory: Option<PathBuf>,
}

impl<T> DirectoryGroupedQueue<T> {
    /// Create a new queue, detecting disk type from the given path.
    pub fn new(sample_path: &Path) -> Self {
        Self {
            by_directory: FxHashMap::default(),
            is_ssd: is_ssd(sample_path),
            current_directory: None,
        }
    }

    /// Create a queue with explicit SSD/HDD mode.
    pub fn with_disk_type(is_ssd: bool) -> Self {
        Self {
            by_directory: FxHashMap::default(),
            is_ssd,
            current_directory: None,
        }
    }

    /// Add an item to the queue.
    pub fn push(&mut self, path: PathBuf, priority: IOPriority, item: T) {
        let parent = path.parent().unwrap_or(&path).to_path_buf();
        self.by_directory
            .entry(parent)
            .or_default()
            .push((priority, item));
    }

    /// Get the next item, optimizing for disk locality on HDDs.
    pub fn pop(&mut self) -> Option<T> {
        if self.by_directory.is_empty() {
            return None;
        }

        if self.is_ssd {
            self.pop_highest_priority()
        } else {
            self.pop_with_locality()
        }
    }

    /// Pop highest priority item regardless of directory (SSD mode).
    fn pop_highest_priority(&mut self) -> Option<T> {
        let best_dir = self
            .by_directory
            .iter()
            .filter(|(_, items)| !items.is_empty())
            .min_by_key(|(_, items)| {
                items
                    .iter()
                    .map(|(p, _)| *p)
                    .min()
                    .unwrap_or(IOPriority::Background)
            })
            .map(|(dir, _)| dir.clone())?;

        self.pop_from_directory(&best_dir)
    }

    /// Pop item with locality preference (HDD mode).
    fn pop_with_locality(&mut self) -> Option<T> {
        if let Some(ref dir) = self.current_directory.clone() {
            if let Some(items) = self.by_directory.get(dir) {
                if !items.is_empty() {
                    return self.pop_from_directory(dir);
                }
            }
        }

        let best_dir = self
            .by_directory
            .iter()
            .filter(|(_, items)| !items.is_empty())
            .min_by_key(|(_, items)| {
                items
                    .iter()
                    .map(|(p, _)| *p)
                    .min()
                    .unwrap_or(IOPriority::Background)
            })
            .map(|(dir, _)| dir.clone())?;

        self.current_directory = Some(best_dir.clone());
        self.pop_from_directory(&best_dir)
    }

    /// Pop highest priority item from a specific directory.
    fn pop_from_directory(&mut self, dir: &PathBuf) -> Option<T> {
        let items = self.by_directory.get_mut(dir)?;

        if items.is_empty() {
            self.by_directory.remove(dir);
            return None;
        }

        let best_idx = items
            .iter()
            .enumerate()
            .min_by_key(|(_, (p, _))| *p)
            .map(|(idx, _)| idx)?;

        let (_, item) = items.swap_remove(best_idx);

        if items.is_empty() {
            self.by_directory.remove(dir);
            if self.current_directory.as_ref() == Some(dir) {
                self.current_directory = None;
            }
        }

        Some(item)
    }

    /// Check if queue is empty.
    pub fn is_empty(&self) -> bool {
        self.by_directory.values().all(|v| v.is_empty())
    }

    /// Get total item count.
    pub fn len(&self) -> usize {
        self.by_directory.values().map(|v| v.len()).sum()
    }
}
