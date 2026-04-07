use crate::app::state::ImageViewerApp;
use crate::workers::global_search_worker::GlobalSearchRequest;

const DEFAULT_GLOBAL_SEARCH_PAGE_LIMIT: u32 = 200;

impl ImageViewerApp {
    pub(crate) fn open_global_search(&mut self) {
        self.global_search.active = true;
        self.global_search.selected_index = None;
        self.global_search.focus_request = true;
        self.global_search.query.clear();
        self.global_search.results.clear();
        self.global_search.results_generation += 1;
        self.global_search.loading = false;
        self.global_search.pending_query_dispatch_at = None;
        self.global_search.in_flight_query = None;
        self.global_search.in_flight_started_at = None;
        self.global_search.has_more_results = false;
        self.global_search.requested_offset = 0;
        self.global_search.requested_limit = DEFAULT_GLOBAL_SEARCH_PAGE_LIMIT;
        self.global_search.scroll_offset_y = 0.0;
        self.global_search.last_scroll_offset_y = 0.0;

        if let Err(error) = self
            .global_search
            .sender
            .send(GlobalSearchRequest::CheckStatus)
        {
            log::error!("[GLOBAL-SEARCH] Failed to queue status check: {}", error);
        }
    }

    pub(crate) fn close_global_search(&mut self) {
        self.global_search.active = false;
        self.global_search.selected_index = None;
        self.global_search.focus_request = false;
        self.global_search.loading = false;
        self.global_search.pending_query_dispatch_at = None;
        self.global_search.in_flight_query = None;
        self.global_search.in_flight_started_at = None;
        self.global_search.has_more_results = false;
        self.global_search.requested_offset = 0;
        self.global_search.requested_limit = DEFAULT_GLOBAL_SEARCH_PAGE_LIMIT;
        self.global_search.scroll_offset_y = 0.0;
        self.global_search.last_scroll_offset_y = 0.0;
        self.global_search.size_cache.clear();
        self.global_search.tooltip_texture_cache.clear();
        self.global_search.metadata_cache.clear();
    }

    pub(crate) fn toggle_global_search(&mut self) {
        if self.global_search.active {
            self.close_global_search();
        } else {
            self.open_global_search();
        }
    }
}
