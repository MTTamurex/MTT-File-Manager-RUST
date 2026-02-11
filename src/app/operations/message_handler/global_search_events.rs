//! Processes responses from the global search worker.

use crate::app::state::ImageViewerApp;
use crate::workers::global_search_worker::GlobalSearchResponse;

impl ImageViewerApp {
    pub(super) fn process_global_search_events(&mut self) {
        while let Ok(response) = self.global_search_receiver.try_recv() {
            match response {
                GlobalSearchResponse::Results { query, items } => {
                    // Only apply if the query still matches (user may have typed more)
                    if query == self.global_search_query {
                        self.global_search_results = items;
                        self.global_search_loading = false;
                    }
                }
                GlobalSearchResponse::Status {
                    available,
                    total_indexed,
                } => {
                    self.global_search_available = available;
                    self.global_search_total_indexed = total_indexed;
                }
                GlobalSearchResponse::Error { query, message } => {
                    if query == self.global_search_query {
                        self.global_search_loading = false;
                    }
                    eprintln!("[GLOBAL-SEARCH] Error for '{}': {}", query, message);
                }
            }
        }

        // Periodically check service availability (every 30 seconds)
        if self.global_search_last_check.elapsed() > std::time::Duration::from_secs(30) {
            self.global_search_last_check = std::time::Instant::now();
            let _ = self.global_search_sender.send(
                crate::workers::global_search_worker::GlobalSearchRequest::CheckStatus,
            );
        }
    }
}
