use super::{BatchSizeRequest, BatchSizeResult, FolderSizeState};
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

pub(super) const MAX_REQUESTS_PER_UPDATE: usize = 30;
pub(super) const MAX_IN_FLIGHT_REQUESTS: usize = 120;

impl FolderSizeState {
    /// Completed values only: display caches can also contain in-flight totals.
    pub(crate) fn export_size_snapshot(&self) -> std::collections::HashMap<String, u64> {
        use crate::domain::file_tag::normalize_tag_path_key;
        let now = std::time::Instant::now();
        let unavailable: std::collections::HashSet<_> = self
            .loading
            .iter()
            .chain(
                self.failed_until
                    .iter()
                    .filter(|(_, until)| **until > now)
                    .map(|(path, _)| path),
            )
            .map(|path| normalize_tag_path_key(path))
            .collect();
        let mut sizes: std::collections::HashMap<_, _> = self
            .size_sorting
            .sizes
            .iter()
            .map(|(path, size)| (normalize_tag_path_key(path), *size))
            .collect();
        for (path, size) in self.batch_cache.iter() {
            let key = normalize_tag_path_key(path);
            if !unavailable.contains(&key) {
                sizes.entry(key).or_insert(*size);
            }
        }
        for (path, summary) in self.cache.iter() {
            let key = normalize_tag_path_key(path);
            if !unavailable.contains(&key) {
                sizes.entry(key).or_insert(summary.total_size);
            }
        }
        for path in self.pending_revalidation.keys() {
            sizes.remove(&normalize_tag_path_key(path));
        }
        sizes
    }

    pub fn fail_batch_requests(&mut self) -> bool {
        let had_pending = !self.batch_loading.is_empty();
        self.batch_loading.clear();
        self.batch_progress.clear();
        self.batch_request_epochs.clear();
        let sorting_changed = self.size_sorting.mark_worker_disconnected();
        let selection_changed = self.selection_size.mark_worker_disconnected();
        had_pending || sorting_changed || selection_changed
    }

    pub fn cache_completed_folder_size(&mut self, path: PathBuf, size: u64) {
        self.display_size_cache.put(path.clone(), size);
        self.batch_progress.remove(&path);
        self.batch_cache.put(path.clone(), size);
        self.size_sorting.complete(&path, size);
        self.selection_size.complete(&path, size);
    }

    pub fn invalidate_calculated_folder_size(&mut self, path: &Path) {
        self.batch_progress.remove(path);
        self.batch_cache.pop(path);
        self.size_sorting.invalidate(path);
        self.selection_size.invalidate(path);
    }

    /// Returns false when already pending or temporarily deferred/capacity-limited.
    pub fn enqueue_batch_size(
        &mut self,
        path: PathBuf,
    ) -> Result<bool, std::sync::mpsc::SendError<BatchSizeRequest>> {
        // All producers share the same bound. Deferred revalidation keeps its
        // first deadline, so continuous notifications cannot postpone retries.
        if self.batch_loading.contains(&path)
            || self.loading.contains(&path)
            || self.pending_revalidation.contains_key(&path)
            || self.batch_loading.len() >= MAX_IN_FLIGHT_REQUESTS
        {
            return Ok(false);
        }
        let generation = self.batch_generation.load(Ordering::Acquire);
        let epoch = self
            .batch_invalidation_epoch
            .get(&path)
            .copied()
            .unwrap_or(0);
        self.batch_req_sender
            .send((path.clone(), generation, epoch))?;
        self.batch_loading.insert(path.clone());
        self.batch_request_epochs.insert(path, epoch);
        Ok(true)
    }

    /// Apply display-only progress or release a terminal request.
    /// Returns true only for a current terminal result.
    /// A previous navigation must not release a newer request for the same path.
    pub fn finish_batch_result(&mut self, result: &BatchSizeResult) -> bool {
        if result.request_generation != self.batch_generation.load(Ordering::Acquire) {
            return false;
        }
        if self
            .batch_request_epochs
            .get(&result.folder_path)
            .is_some_and(|epoch| result.request_epoch < *epoch)
        {
            return false;
        }
        if !result.complete {
            if self.batch_loading.contains(&result.folder_path)
                && !self.loading.contains(&result.folder_path)
                && self.batch_request_epochs.get(&result.folder_path) == Some(&result.request_epoch)
                && result.request_epoch
                    >= self
                        .batch_invalidation_epoch
                        .get(&result.folder_path)
                        .copied()
                        .unwrap_or(0)
            {
                if let Some(size) = result.total_size {
                    let partial = self
                        .batch_progress
                        .entry(result.folder_path.clone())
                        .or_default();
                    *partial = (*partial).max(size);
                }
            }
            return false;
        }
        self.batch_progress.remove(&result.folder_path);
        self.batch_loading.remove(&result.folder_path);
        self.batch_request_epochs.remove(&result.folder_path);
        result.request_epoch
            >= self
                .batch_invalidation_epoch
                .get(&result.folder_path)
                .copied()
                .unwrap_or(0)
    }
}
