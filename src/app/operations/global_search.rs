use crate::app::global_search_state::{GlobalSearchInteractionTarget, GlobalSearchTagFilter};
use crate::app::initial_indexing_notice::InitialIndexingNoticeEvent;
use crate::app::state::ImageViewerApp;
use crate::infrastructure::app_state_db::PreferenceWriteOutcome;
use crate::workers::global_search_worker::GlobalSearchRequest;

const DEFAULT_GLOBAL_SEARCH_PAGE_LIMIT: u32 = 500;
const INITIAL_INDEXING_NOTIFICATION_KEY: &str = "initial_indexing";
const INITIAL_INDEXING_COMPLETED_GENERATION_KEY: &str = "initial_indexing_completed_generation";

impl ImageViewerApp {
    pub(crate) fn request_global_search_refresh(&mut self) {
        if !self.global_search.active || self.global_search.query.is_empty() {
            return;
        }
        self.global_search.pending_query_dispatch_at =
            Some(std::time::Instant::now() + std::time::Duration::from_millis(500));
        self.ui_ctx
            .request_repaint_after(std::time::Duration::from_millis(500));
    }

    pub(crate) fn open_global_search(&mut self) {
        self.context_menu.close();
        self.global_search.active = true;
        self.global_search.opened_at = std::time::Instant::now();
        self.global_search.focus_request = true;
        self.global_search.interaction_target = GlobalSearchInteractionTarget::SearchInput;
        self.global_search.rename_state = None;
        self.global_search.suspended_for_drag = false;
        self.global_search.shell_refresh_request_id = None;
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
        self.global_search.created_after_text.clear();
        self.global_search.created_before_text.clear();
        self.global_search.tag_filter = GlobalSearchTagFilter::All;

        self.sync_global_search_status_tracking();
        self.request_global_search_status_refresh();
    }

    pub(crate) fn close_global_search(&mut self) {
        if self.context_menu.origin
            == crate::application::context_menu::ContextMenuOrigin::GlobalSearch
        {
            self.context_menu.close();
        }
        self.global_search.active = false;
        self.global_search.release_transient_results();
        self.global_search.clear_transient_caches();
        self.global_search.focus_request = false;
        self.global_search.rename_state = None;
        self.global_search.suspended_for_drag = false;
        self.global_search.shell_refresh_request_id = None;
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

        self.sync_global_search_status_tracking();
    }

    pub(crate) fn toggle_global_search(&mut self) {
        if self.global_search.active {
            self.close_global_search();
        } else {
            self.open_global_search();
        }
    }

    pub(crate) fn start_initial_indexing_notice(&mut self) {
        if !self.initial_indexing_notice.is_tracking() {
            return;
        }

        self.notifications.persistent_info(
            INITIAL_INDEXING_NOTIFICATION_KEY,
            rust_i18n::t!("notifications.initial_indexing_preparing").to_string(),
        );
        self.sync_global_search_status_tracking();
        self.request_global_search_status_refresh();
    }

    pub(crate) fn update_initial_indexing_notice(&mut self) {
        let event = self.initial_indexing_notice.observe_status(
            self.global_search.available,
            self.global_search.total_indexed,
            &self.global_search.status_volumes,
        );

        match event {
            InitialIndexingNoticeEvent::None => {}
            InitialIndexingNoticeEvent::ProgressChanged => {
                let message = if self.global_search.total_indexed == 0 {
                    rust_i18n::t!("notifications.initial_indexing_preparing").to_string()
                } else {
                    rust_i18n::t!(
                        "notifications.initial_indexing_progress",
                        count = self.global_search.total_indexed
                    )
                    .to_string()
                };
                self.notifications
                    .persistent_info(INITIAL_INDEXING_NOTIFICATION_KEY, message);
            }
            InitialIndexingNoticeEvent::Completed => {
                self.notifications.success_replacing(
                    INITIAL_INDEXING_NOTIFICATION_KEY,
                    rust_i18n::t!("notifications.initial_indexing_completed").to_string(),
                );
                self.sync_global_search_status_tracking();
            }
        }

        self.persist_initial_indexing_completion();
    }

    pub(crate) fn persist_initial_indexing_completion(&mut self) {
        let Some(generation) = self
            .initial_indexing_notice
            .completion_generation()
            .map(str::to_owned)
        else {
            return;
        };

        match self
            .app_state_db
            .try_set_preference(INITIAL_INDEXING_COMPLETED_GENERATION_KEY, &generation)
        {
            PreferenceWriteOutcome::Persisted => self
                .initial_indexing_notice
                .mark_completion_persistence_finished(),
            PreferenceWriteOutcome::Busy => {
                self.ui_ctx
                    .request_repaint_after(std::time::Duration::from_millis(100));
            }
            PreferenceWriteOutcome::Failed(error) => {
                log::warn!("[INITIAL-INDEXING] Failed to persist completion: {error}");
                self.initial_indexing_notice
                    .mark_completion_persistence_finished();
            }
        }
    }

    fn sync_global_search_status_tracking(&self) {
        let active = self.global_search.active || self.initial_indexing_notice.is_tracking();
        if let Err(error) = self
            .global_search
            .sender
            .send(GlobalSearchRequest::SetStatusTracking { active })
        {
            log::error!("[GLOBAL-SEARCH] Failed to update status tracking: {error}");
        }
    }

    fn request_global_search_status_refresh(&self) {
        if let Err(error) = self
            .global_search
            .sender
            .send(GlobalSearchRequest::CheckStatus)
        {
            log::error!("[GLOBAL-SEARCH] Failed to queue status check: {error}");
        }
    }
}
