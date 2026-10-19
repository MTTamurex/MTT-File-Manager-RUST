//! Listing-scoped sizes must survive eviction from the display LRU cache.
use super::FolderSizeState;
use crate::domain::file_entry::{is_archive_extension, FileEntry};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct SizeSortingState {
    pub sizes: HashMap<PathBuf, u64>,
    tracked: rustc_hash::FxHashSet<PathBuf>,
    dirty: bool,
    next_update: Option<Instant>,
    sorted_listings: Vec<Weak<Vec<FileEntry>>>,
    worker_disconnected: bool,
}

impl SizeSortingState {
    pub fn has_pending_sizes(&self) -> bool {
        self.sizes.len() < self.tracked.len()
    }

    pub fn needs_update(&self) -> bool {
        (!self.worker_disconnected && self.has_pending_sizes()) || self.dirty
    }

    pub fn is_sorted(&self, items: &Arc<Vec<FileEntry>>) -> bool {
        let items = Arc::downgrade(items);
        self.sorted_listings
            .iter()
            .any(|listing| listing.ptr_eq(&items))
    }

    pub fn remember_sorted(&mut self, items: &Arc<Vec<FileEntry>>) {
        self.sorted_listings
            .retain(|listing| listing.strong_count() > 0);
        if !self.is_sorted(items) {
            if self.sorted_listings.len() == 2 {
                self.sorted_listings.remove(0);
            }
            self.sorted_listings.push(Arc::downgrade(items));
        }
    }

    pub fn forget_sorted_listings(&mut self) {
        self.sorted_listings.clear();
    }

    pub fn complete(&mut self, path: &Path, size: u64) {
        if self.tracked.contains(path) && self.sizes.insert(path.to_path_buf(), size) != Some(size)
        {
            self.dirty = true;
        }
    }

    pub fn invalidate(&mut self, path: &Path) {
        self.dirty |= self.sizes.remove(path).is_some();
    }
}

impl FolderSizeState {
    /// Queue the entire listing, not only rows painted by the list renderer.
    /// Bound queued work and update frequency to keep the UI responsive.
    #[cfg(test)]
    pub fn prepare_size_sort(&mut self, items: &[FileEntry], now: Instant, force: bool) -> bool {
        self.prepare_size_sort_for_listings(items, &[], now, force)
    }

    pub fn prepare_size_sort_for_listings(
        &mut self,
        items: &[FileEntry],
        other_items: &[FileEntry],
        now: Instant,
        force: bool,
    ) -> bool {
        if !force && self.size_sorting.next_update.is_some_and(|next| now < next) {
            return false;
        }
        self.size_sorting.next_update = Some(now + Duration::from_millis(100));
        self.size_sorting.tracked.clear();
        self.size_sorting.tracked.extend(
            items
                .iter()
                .chain(other_items)
                .filter(|item| {
                    item.is_dir
                        && !is_archive_extension(&item.name)
                        && item.drive_info.is_none()
                        && !item.is_recycle_item()
                })
                .map(|item| item.path.clone()),
        );
        let tracked = &self.size_sorting.tracked;
        self.size_sorting
            .sizes
            .retain(|path, _| tracked.contains(path));

        let mut requests = Vec::new();
        for path in &self.size_sorting.tracked {
            if self.size_sorting.sizes.contains_key(path) {
                continue;
            }
            // Preview progress is also written to batch_cache; don't mistake
            // an in-progress total for a completed directory size.
            if !self.loading.contains(path) {
                if let Some(size) = self.batch_cache.peek(path).copied() {
                    self.size_sorting.sizes.insert(path.clone(), size);
                    self.size_sorting.dirty = true;
                    continue;
                }
            }
            if self.batch_loading.contains(path)
                || self.loading.contains(path)
                || self.pending_revalidation.contains_key(path)
                || requests.len() >= 30
                || self.batch_loading.len() + requests.len() >= 120
                || self.size_sorting.worker_disconnected
            {
                continue;
            }
            requests.push(path.clone());
        }
        for path in requests {
            if self.enqueue_batch_size(path).is_err() {
                self.size_sorting.worker_disconnected = true;
                break;
            }
        }
        std::mem::take(&mut self.size_sorting.dirty)
    }
}
