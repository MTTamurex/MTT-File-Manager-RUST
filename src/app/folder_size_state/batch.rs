use super::{BatchSizeRequest, BatchSizeResult, FolderSizeState};
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

impl FolderSizeState {
    pub fn fail_batch_requests(&mut self) -> bool {
        let had_pending = !self.batch_loading.is_empty();
        self.batch_loading.clear();
        self.batch_request_epochs.clear();
        let sorting_changed = self.size_sorting.mark_worker_disconnected();
        let selection_changed = self.selection_size.mark_worker_disconnected();
        had_pending || sorting_changed || selection_changed
    }

    pub fn cache_completed_folder_size(&mut self, path: PathBuf, size: u64) {
        self.batch_cache.put(path.clone(), size);
        self.size_sorting.complete(&path, size);
        self.selection_size.complete(&path, size);
    }

    pub fn invalidate_calculated_folder_size(&mut self, path: &Path) {
        self.batch_cache.pop(path);
        self.size_sorting.invalidate(path);
        self.selection_size.invalidate(path);
    }

    pub fn enqueue_batch_size(
        &mut self,
        path: PathBuf,
    ) -> Result<(), std::sync::mpsc::SendError<BatchSizeRequest>> {
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
        Ok(())
    }

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
