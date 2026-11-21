use crate::app::ImageViewerApp;

impl ImageViewerApp {
    pub(crate) fn process_drive_indexing_events(&mut self) {
        let changes = self.drive_indexing.poll();
        if changes.cache_changed {
            self.save_preferences();
        }
        if changes.policy_changed {
            self.folder_size_state
                .invalidate_indexing_drives(changes.changed_drive_mask);
            self.global_search.clear_transient_results();
            self.global_search.clear_transient_caches();
            self.global_search.loading = false;
            self.global_search.in_flight_query = None;
            self.global_search.in_flight_started_at = None;
            self.global_search.requested_offset = 0;
            self.request_global_search_refresh();
        }
    }
}
