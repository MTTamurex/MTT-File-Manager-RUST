use super::*;

impl ImageViewerApp {
    /// NTFS watchers observe listing changes, not every recursive size change.
    /// Reuse the background worker for index-only checks in the focused pane.
    pub(super) fn maybe_send_indexed_folder_size_probe(&mut self) {
        if !is_filesystem_probe_target(&self.navigation_state.current_path)
            || self.is_loading_folder
            || self.file_operation_state.file_ops_in_progress > 0
            || self.layout.saved_is_minimized
            || self.thumbnail_scroll_is_settling()
            || self.folder_size_state.content_probe_pending.is_some()
            || self.watcher_fallback_last_probe.elapsed() < INACTIVE_PANEL_PROBE_INTERVAL
            || (self.minimized_duration_secs >= 10.0
                && self.last_restore_time.elapsed() < Duration::from_secs(8))
        {
            return;
        }

        let path = PathBuf::from(&self.navigation_state.current_path);
        if crate::infrastructure::onedrive::is_cloud_sync_path(&path) {
            return;
        }
        self.watcher_fallback_last_probe = Instant::now();
        let Some((pending, threshold)) = self
            .folder_size_state
            .prepare_content_probe(&path, Instant::now())
        else {
            return;
        };
        let request = ConsistencyProbeRequest {
            path: path.clone(),
            is_onedrive: false,
            ui_signature: 0,
            show_hidden_files: self.show_hidden_files,
            mode: ConsistencyProbeMode::FolderContents,
            modified_threshold_secs: threshold,
            folder_cover_states: pending
                .paths
                .iter()
                .cloned()
                .map(|path| (path, None))
                .collect(),
        };
        if self.consistency_probe_tx.send(request).is_ok() {
            self.folder_size_state.content_probe_pending = Some(pending);
        }
    }

    pub(super) fn apply_indexed_folder_size_probe_result(
        &mut self,
        result: ConsistencyProbeResult,
    ) {
        if !self
            .folder_size_state
            .finish_content_probe(&result.path, result.content_check_succeeded)
        {
            return;
        }
        // This mode has no disk listing signature or disappearance information.
        // Never treat it as listing drift. Even after navigation, invalidate the
        // queried cached sizes so returning to that folder cannot reuse old totals.
        let paths = result.changed_folder_contents;
        if paths.is_empty() {
            return;
        }
        // Preserve navigation-cache correctness without SQLite DELETEs on the UI
        // thread. Dirty folders bypass the persistent index when next loaded.
        for path in &paths {
            self.directory_dirty_registry.mark_dirty(path);
            self.directory_cache.invalidate(path);
            self.miller_columns.invalidate(path);
            if let Some(snapshot) = self.dual_panel_inactive_state.as_mut() {
                snapshot.miller_columns.invalidate(path);
            }
            self.invalidate_folder_size_cache(path);
        }
        self.invalidate_content_ancestor_sizes(&paths);
        super::super::panel_tab_cache::invalidate_cached_panel_tab_listings_for_paths(self, &paths);
        self.ui_ctx.request_repaint();
    }
}
