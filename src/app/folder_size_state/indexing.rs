//! Conservative per-drive cache eviction, without filesystem queries on the UI.
//! It does not disable the independent non-NTFS recursive calculation path.
use super::*;
use std::path::Path;

pub(super) fn matches_drive(path: &Path, mask: u32) -> bool {
    crate::infrastructure::drive_indexing::path_drive_letter(path)
        .is_some_and(|letter| mask & (1 << (letter as u32 - 'A' as u32)) != 0)
}

impl FolderSizeState {
    pub(crate) fn invalidate_indexing_drives(&mut self, mask: u32) {
        let mut paths = FxHashSet::default();
        for path in self
            .cache
            .iter()
            .map(|(path, _)| path)
            .chain(self.panel_stale_cache.iter().map(|(path, _)| path))
            .chain(self.batch_cache.iter().map(|(path, _)| path))
            .chain(self.display_size_cache.iter().map(|(path, _)| path))
            .chain(self.loading.iter())
            .chain(self.active_requests.keys())
            .chain(self.batch_loading.iter())
            .chain(self.batch_progress.keys())
            .chain(self.batch_request_epochs.keys())
            .chain(self.batch_invalidation_epoch.keys())
            .chain(self.panel_deferred_revalidation.keys())
            .chain(self.pending_revalidation.keys())
            .chain(self.failed_until.keys())
            .chain(self.size_sorting.sizes.keys())
        {
            if matches_drive(path, mask) {
                paths.insert(path.clone());
            }
        }
        for path in paths {
            self.cancel_active_request(&path);
            self.cache.pop(&path);
            self.invalidate_calculated_folder_size(&path);
            self.panel_stale_cache.pop(&path);
            self.display_size_cache.pop(&path);
            self.panel_deferred_revalidation.remove(&path);
            let had_pending_batch =
                self.batch_loading.remove(&path) || self.batch_request_epochs.contains_key(&path);
            self.failed_until.remove(&path);
            self.cancel_revalidation(&path);
            let epoch = self
                .batch_invalidation_epoch
                .entry(path.clone())
                .or_default();
            *epoch = epoch.wrapping_add(1);
            if had_pending_batch {
                // Keep a rejection watermark even while the UI may enqueue a
                // replacement. It lives until a current terminal result or
                // navigation, not merely until the usual two-second pruning.
                self.batch_request_epochs.insert(path, *epoch);
            }
        }
        self.selection_size.invalidate_indexing_drives(mask);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_indexing_change_evicts_all_affected_displays_but_keeps_other_drives() {
        let mut state = super::super::tests::test_state();
        let affected = PathBuf::from(r"R:\folder");
        let other = PathBuf::from(r"S:\folder");
        for path in [&affected, &other] {
            let summary = FolderContentSummary::complete(100, 1, 1);
            state.cache.put(path.clone(), summary);
            state.cache_completed_panel_summary(path.clone(), summary);
            state.cache_completed_folder_size(path.clone(), 100);
        }
        state.invalidate_indexing_drives(1 << ('R' as u32 - 'A' as u32));
        assert!(state.cache.peek(&affected).is_none());
        assert!(state.panel_stale_cache.peek(&affected).is_none());
        assert!(state.display_size_cache.peek(&affected).is_none());
        assert!(state.batch_cache.peek(&affected).is_none());
        assert!(state.cache.peek(&other).is_some());
        assert!(state.panel_stale_cache.peek(&other).is_some());
        assert!(state.display_size_cache.peek(&other).is_some());
    }

    #[test]
    fn drive_indexing_change_rejects_single_and_batch_results_already_queued_for_ui() {
        let mut state = super::super::tests::test_state();
        let path = PathBuf::from(r"R:\folder");
        state.active_requests.insert(
            path.clone(),
            ActiveFolderSizeRequest {
                request_id: 7,
                deadline: Instant::now() + Duration::from_secs(30),
            },
        );
        state.loading.insert(path.clone());
        state.batch_loading.insert(path.clone());
        state.batch_request_epochs.insert(path.clone(), 0);
        let result = BatchSizeResult {
            folder_path: path.clone(),
            summary: Some(FolderContentSummary::complete(100, 1, 1)),
            complete: true,
            cancelled: false,
            request_epoch: 0,
            request_generation: state.batch_generation.load(Ordering::Acquire),
        };
        state.invalidate_indexing_drives(1 << ('R' as u32 - 'A' as u32));
        state.prune_stale_invalidation_epochs(Instant::now() + Duration::from_secs(10));
        assert!(!state.is_active_request(&path, 7));
        assert!(!state.finish_batch_result(&result));
        assert!(state.cache.peek(&path).is_none());
        assert!(state.batch_cache.peek(&path).is_none());
    }

    #[test]
    fn drive_indexing_rejection_watermarks_are_released_by_current_terminal_results() {
        let mut state = super::super::tests::test_state();
        let path = PathBuf::from(r"R:\folder");
        state.batch_loading.insert(path.clone());
        state.batch_request_epochs.insert(path.clone(), 0);
        state.invalidate_indexing_drives(1 << ('R' as u32 - 'A' as u32));
        let current = state.batch_request_epochs[&path];
        let result = BatchSizeResult {
            folder_path: path.clone(),
            summary: None,
            complete: true,
            cancelled: false,
            request_epoch: current,
            request_generation: state.batch_generation.load(Ordering::Acquire),
        };
        assert!(state.finish_batch_result(&result));
        state.prune_stale_invalidation_epochs(Instant::now() + Duration::from_secs(10));
        assert!(!state.batch_request_epochs.contains_key(&path));
        assert!(!state.batch_invalidation_epoch.contains_key(&path));
    }
}
