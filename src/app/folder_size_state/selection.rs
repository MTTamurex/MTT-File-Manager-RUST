//! Aggregate selected files and completed directory sizes without blocking the UI.
use super::FolderSizeState;
use crate::domain::file_entry::FileEntry;
use crate::ui::cache::FxHashSet;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectionSizeSummary {
    pub total_size: u64,
    pub pending_folders: usize,
    pub unavailable_items: usize,
}

#[derive(Clone, Copy)]
enum FolderSize {
    Pending,
    Complete(u64),
    Unavailable,
}

#[derive(Default)]
pub struct SelectionSizeState {
    folders: HashMap<PathBuf, FolderSize>,
    worker_disconnected: bool,
}

impl SelectionSizeState {
    pub fn mark_worker_disconnected(&mut self) -> bool {
        let changed = !self.worker_disconnected;
        self.worker_disconnected = true;
        for value in self.folders.values_mut() {
            if matches!(value, FolderSize::Pending) {
                *value = FolderSize::Unavailable;
            }
        }
        changed
    }

    pub fn complete(&mut self, path: &Path, size: u64) {
        if let Some(value) = self.folders.get_mut(path) {
            *value = FolderSize::Complete(size);
        }
    }

    pub fn invalidate(&mut self, path: &Path) {
        if let Some(value) = self.folders.get_mut(path) {
            *value = FolderSize::Pending;
        }
    }

    pub fn failed(&mut self, path: &Path) {
        if let Some(value) = self.folders.get_mut(path) {
            *value = FolderSize::Unavailable;
        }
    }
}

impl FolderSizeState {
    pub fn selection_size(
        &mut self,
        items: &[FileEntry],
        selected: &FxHashSet<PathBuf>,
        now: Instant,
    ) -> SelectionSizeSummary {
        if selected.len() <= 1 {
            self.selection_size.folders.clear();
            return SelectionSizeSummary::default();
        }
        self.selection_size
            .folders
            .retain(|path, _| selected.contains(path));
        let mut summary = SelectionSizeSummary::default();
        let mut requested = 0;
        let mut remaining: FxHashSet<_> = selected.iter().collect();
        for item in items {
            if remaining.is_empty() {
                break;
            }
            if !remaining.remove(&item.path) {
                continue;
            }
            // These are virtual entries, not directories that the batch worker
            // can traverse. Never report their zero metadata size as a total.
            if item.is_dir
                && !item.is_archive()
                && (crate::domain::file_entry::is_path_inside_archive(&item.path)
                    || crate::domain::special_paths::is_virtual_path(&item.path.to_string_lossy()))
            {
                summary.unavailable_items += 1;
                continue;
            }
            if !item.is_dir
                || item.is_archive()
                || item.is_recycle_item()
                || item.drive_info.is_some()
            {
                summary.total_size = summary.total_size.saturating_add(item.size);
                continue;
            }
            let mut value = self
                .selection_size
                .folders
                .get(&item.path)
                .copied()
                .unwrap_or(FolderSize::Pending);
            let preview_loading = self.loading.contains(&item.path);
            let preview_failed = self
                .failed_until
                .get(&item.path)
                .is_some_and(|until| now < *until);
            if !matches!(value, FolderSize::Complete(_)) {
                if let Some(size) = self
                    .size_sorting
                    .sizes
                    .get(&item.path)
                    .or_else(|| {
                        // Preview progress shares the display cache. A partial
                        // value is never a completed selection total.
                        if preview_loading || preview_failed {
                            None
                        } else {
                            self.batch_cache.peek(&item.path)
                        }
                    })
                    .copied()
                {
                    value = FolderSize::Complete(size);
                }
            }
            if !matches!(value, FolderSize::Complete(_)) {
                if preview_failed {
                    value = FolderSize::Unavailable;
                } else if preview_loading {
                    // The independent preview worker may still finish even if
                    // the batch worker has stopped.
                    value = FolderSize::Pending;
                } else if self.selection_size.worker_disconnected {
                    value = FolderSize::Unavailable;
                } else if !self.batch_loading.contains(&item.path)
                    && !self.pending_revalidation.contains_key(&item.path)
                    && requested < 30
                    && self.batch_loading.len() < 120
                {
                    if self.enqueue_batch_size(item.path.clone()).is_ok() {
                        requested += 1;
                        value = FolderSize::Pending;
                    } else {
                        self.selection_size.worker_disconnected = true;
                        value = FolderSize::Unavailable;
                    }
                }
            }
            if let Some(current) = self.selection_size.folders.get_mut(&item.path) {
                *current = value;
            } else {
                self.selection_size.folders.insert(item.path.clone(), value);
            }
            match value {
                FolderSize::Complete(size) => {
                    summary.total_size = summary.total_size.saturating_add(size)
                }
                FolderSize::Pending => summary.pending_folders += 1,
                FolderSize::Unavailable => summary.unavailable_items += 1,
            }
        }
        summary.unavailable_items += remaining.len();
        summary
    }
}
