use super::FolderSizeState;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const FOLDER_SIZE_REVALIDATION_INITIAL_DELAY: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy)]
pub struct PendingFolderSizeRevalidation {
    deadline: Instant,
    release_batch_loading: bool,
}

impl FolderSizeState {
    pub fn should_prune_pending_revalidations(&self, now: Instant) -> bool {
        self.pending_revalidation_next_deadline
            .is_some_and(|deadline| deadline <= now)
    }

    pub fn schedule_revalidation(&mut self, folder_path: PathBuf, now: Instant) -> Duration {
        // Keep the first deadline and any failed-request release flag. Watcher
        // events must not postpone the retry or strand a path in batch_loading.
        let deadline = self
            .pending_revalidation
            .entry(folder_path)
            .or_insert(PendingFolderSizeRevalidation {
                deadline: now + FOLDER_SIZE_REVALIDATION_INITIAL_DELAY,
                release_batch_loading: false,
            })
            .deadline;
        self.pending_revalidation_next_deadline = Some(
            self.pending_revalidation_next_deadline
                .map_or(deadline, |current| current.min(deadline)),
        );
        deadline.saturating_duration_since(now)
    }

    pub fn schedule_revalidation_if_absent(
        &mut self,
        folder_path: PathBuf,
        now: Instant,
    ) -> Duration {
        let delay = self.schedule_revalidation(folder_path.clone(), now);
        self.pending_revalidation
            .get_mut(&folder_path)
            .unwrap()
            .release_batch_loading = true;
        delay
    }

    pub fn cancel_revalidation(&mut self, folder_path: &PathBuf) {
        if self.pending_revalidation.remove(folder_path).is_some()
            && self.pending_revalidation.is_empty()
        {
            self.pending_revalidation_next_deadline = None;
        }
    }

    pub fn cancel_revalidations(&mut self, folder_paths: &[PathBuf]) {
        for path in folder_paths {
            self.cancel_revalidation(path);
        }
    }

    pub fn take_expired_revalidations(&mut self, now: Instant) -> Vec<(PathBuf, bool)> {
        let mut expired = Vec::new();
        self.pending_revalidation.retain(|path, revalidation| {
            let keep = revalidation.deadline > now;
            if !keep {
                expired.push((path.clone(), revalidation.release_batch_loading));
            }
            keep
        });
        self.pending_revalidation_next_deadline = self
            .pending_revalidation
            .values()
            .map(|entry| entry.deadline)
            .min();
        expired
    }
}
