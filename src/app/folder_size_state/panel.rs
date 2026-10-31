//! Confirmed details-panel summaries and bounded, silent revalidation.
use super::*;

impl FolderSizeState {
    pub fn cache_completed_panel_summary(&mut self, path: PathBuf, summary: FolderContentSummary) {
        if summary.has_counts() {
            self.panel_stale_cache.put(path.clone(), summary);
            self.panel_deferred_revalidation.remove(&path);
            self.prune_panel_revalidations_without_stale();
        }
    }

    pub fn reschedule_panel_revalidation_if_stale(&mut self, path: &PathBuf, now: Instant) {
        if self.panel_stale_cache.contains(path) {
            // Repeated changes must not keep pushing the deadline forward.
            self.panel_deferred_revalidation
                .entry(path.clone())
                .or_insert(now + PANEL_STALE_REVALIDATION_DELAY);
        }
    }

    pub fn summary_for_panel_render(&self, path: &PathBuf) -> (Option<FolderContentSummary>, bool) {
        if let Some(summary) = self.panel_stale_cache.peek(path).copied() {
            // Display only: request scheduling uses the real loading/pending state.
            // Neither progress nor a size-only result replaces a confirmed triplet.
            return (Some(summary), false);
        }
        (self.cache.peek(path).copied(), self.loading.contains(path))
    }

    pub fn take_due_panel_revalidation(&mut self, now: Instant, path: &PathBuf) -> Option<PathBuf> {
        let deadline = self.panel_deferred_revalidation.get(path).copied()?;
        if deadline > now
            || self.loading.contains(path)
            || self.pending_revalidation.contains_key(path)
            || self.is_failure_active(path, now)
        {
            return None;
        }
        self.panel_deferred_revalidation.remove(path);
        self.panel_stale_cache.contains(path).then(|| path.clone())
    }

    /// All automatic details-panel requests share deduplication, debounce and
    /// failure backoff. Deferring a request must not cancel another active scan.
    pub fn request_panel_refresh(&mut self, path: PathBuf, now: Instant) -> bool {
        if self.loading.contains(&path)
            || self.pending_revalidation.contains_key(&path)
            || self
                .panel_deferred_revalidation
                .get(&path)
                .is_some_and(|due| *due > now)
            || self.is_failure_active(&path, now)
        {
            return false;
        }
        let epoch = self
            .batch_invalidation_epoch
            .get(&path)
            .copied()
            .unwrap_or(0);
        self.cancel.store(true, Ordering::Release);
        self.dispatch_request(path, epoch, now)
    }

    fn prune_panel_revalidations_without_stale(&mut self) {
        let cache = &self.panel_stale_cache;
        self.panel_deferred_revalidation
            .retain(|path, _| cache.contains(path));
    }
}

#[cfg(test)]
mod tests;
