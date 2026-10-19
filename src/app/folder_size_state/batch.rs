use super::{BatchSizeRequest, BatchSizeResult, FolderSizeState};
use std::path::PathBuf;
use std::sync::atomic::Ordering;

impl FolderSizeState {
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
