use crate::app::global_search_state::GlobalSearchTagFilter;
use crate::app::state::ImageViewerApp;
use crate::workers::global_search_worker::GlobalSearchRequest;

const DEFAULT_GLOBAL_SEARCH_PAGE_LIMIT: u32 = 500;

impl ImageViewerApp {
    pub(crate) fn open_global_search(&mut self) {
        self.global_search.active = true;
        self.global_search.opened_at = std::time::Instant::now();
        self.global_search.focus_request = true;
        self.global_search.query.clear();
        self.global_search.clear_transient_results();
        self.global_search.clear_transient_caches();
        self.global_search.loading = false;
        self.global_search.pending_query_dispatch_at = None;
        self.global_search.in_flight_query = None;
        self.global_search.in_flight_started_at = None;
        self.global_search.last_status_received_at = std::time::Instant::now();
        self.global_search.last_progress_advance_at = std::time::Instant::now();
        self.global_search.requested_offset = 0;
        self.global_search.requested_limit = DEFAULT_GLOBAL_SEARCH_PAGE_LIMIT;
        self.global_search.scroll_offset_y = 0.0;
        self.global_search.last_scroll_offset_y = 0.0;
        self.global_search.session_total_indexed = 0;
        self.global_search.category = crate::app::global_search_state::GlobalSearchCategory::All;
        self.global_search.drive_filter = None;
        self.global_search.sort_mode =
            crate::app::global_search_state::GlobalSearchSortMode::Relevance;
        self.global_search.sort_descending = false;
        self.global_search.min_size_mb = None;
        self.global_search.max_size_mb = None;
        self.global_search.created_after = None;
        self.global_search.created_before = None;
        self.global_search.created_after_month = 0;
        self.global_search.created_after_day = 0;
        self.global_search.created_after_year = 0;
        self.global_search.created_after_month_text.clear();
        self.global_search.created_after_day_text.clear();
        self.global_search.created_after_year_text.clear();
        self.global_search.created_before_month = 0;
        self.global_search.created_before_day = 0;
        self.global_search.created_before_year = 0;
        self.global_search.created_before_month_text.clear();
        self.global_search.created_before_day_text.clear();
        self.global_search.created_before_year_text.clear();
        self.global_search.tag_filter = GlobalSearchTagFilter::All;

        if let Err(error) = self
            .global_search
            .sender
            .send(GlobalSearchRequest::SetStatusTracking { active: true })
        {
            log::error!(
                "[GLOBAL-SEARCH] Failed to enable status tracking: {}",
                error
            );
        }

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
        self.global_search.release_transient_results();
        self.global_search.clear_transient_caches();
        self.global_search.focus_request = false;
        self.global_search.loading = false;
        self.global_search.pending_query_dispatch_at = None;
        self.global_search.in_flight_query = None;
        self.global_search.in_flight_started_at = None;
        self.global_search.requested_offset = 0;
        self.global_search.requested_limit = DEFAULT_GLOBAL_SEARCH_PAGE_LIMIT;
        self.global_search.scroll_offset_y = 0.0;
        self.global_search.last_scroll_offset_y = 0.0;
        self.global_search.session_total_indexed = 0;
        self.global_search.tag_filter = GlobalSearchTagFilter::All;

        if let Err(error) = self
            .global_search
            .sender
            .send(GlobalSearchRequest::SetStatusTracking { active: false })
        {
            log::error!(
                "[GLOBAL-SEARCH] Failed to disable status tracking: {}",
                error
            );
        }
    }

    pub(crate) fn toggle_global_search(&mut self) {
        if self.global_search.active {
            self.close_global_search();
        } else {
            self.open_global_search();
        }
    }
}
