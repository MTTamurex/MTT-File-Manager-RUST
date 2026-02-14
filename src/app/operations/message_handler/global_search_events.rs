//! Processes responses from the global search worker.

use crate::app::state::ImageViewerApp;
use crate::workers::global_search_worker::GlobalSearchResponse;

impl ImageViewerApp {
    pub(super) fn process_global_search_events(&mut self) {
        while let Ok(response) = self.global_search.receiver.try_recv() {
            match response {
                GlobalSearchResponse::Results {
                    query,
                    items,
                    offset,
                    limit,
                    has_more,
                } => {
                    // Only apply if the query still matches (user may have typed more)
                    if query == self.global_search.query {
                        if offset == 0 {
                            self.global_search.results = items;
                            self.global_search.selected_index = None;
                        } else if offset == self.global_search.results.len() as u32 {
                            append_unique_results(&mut self.global_search.results, items);
                        } else {
                            // Stale page response (offset mismatch), ignore.
                            continue;
                        }

                        self.global_search.loading = false;
                        self.global_search.requested_offset = offset;
                        self.global_search.requested_limit = limit;
                        self.global_search.has_more_results = has_more;
                    }
                }
                GlobalSearchResponse::Status {
                    available,
                    total_indexed,
                } => {
                    self.global_search.available = available;
                    self.global_search.total_indexed = total_indexed;
                }
                GlobalSearchResponse::Error { query, message } => {
                    if query == self.global_search.query {
                        self.global_search.loading = false;
                        self.global_search.has_more_results = false;
                    }
                    log::error!("[GLOBAL-SEARCH] Error for '{}': {}", query, message);

                    // Service IPC can be temporarily unstable after app/service restart.
                    // Trigger an expedited status check to recover UI state quickly.
                    if is_connectivity_error(&message)
                        && self.global_search.last_check.elapsed()
                            > std::time::Duration::from_secs(1)
                    {
                        self.global_search.last_check = std::time::Instant::now();
                        let _ = self.global_search.sender.send(
                            crate::workers::global_search_worker::GlobalSearchRequest::CheckStatus,
                        );
                    }
                }
            }
        }

        // Check availability faster while offline, slower while stable online.
        let interval = if self.global_search.active {
            std::time::Duration::from_secs(1)
        } else if self.global_search.available {
            std::time::Duration::from_secs(30)
        } else {
            std::time::Duration::from_secs(3)
        };

        if self.global_search.last_check.elapsed() > interval {
            self.global_search.last_check = std::time::Instant::now();
            let _ = self
                .global_search
                .sender
                .send(crate::workers::global_search_worker::GlobalSearchRequest::CheckStatus);
        }
    }
}

fn is_connectivity_error(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("search service not available")
        || m.contains("all pipe instances are busy")
        || m.contains("no process is on the other end of the pipe")
        || m.contains("pipe closed during read")
        || m.contains("peeknamedpipe failed")
        || m.contains("writefile failed")
        || m.contains("readfile failed")
        || m.contains("timeout")
}

fn normalize_result_path(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    let stripped = lower.strip_prefix(r"\\?\").unwrap_or(&lower);

    if stripped.len() > 3 {
        stripped.trim_end_matches('\\').to_string()
    } else {
        stripped.to_string()
    }
}

fn append_unique_results(
    target: &mut Vec<mtt_search_protocol::SearchResultItem>,
    extra: Vec<mtt_search_protocol::SearchResultItem>,
) {
    if extra.is_empty() {
        return;
    }

    let mut seen = std::collections::HashSet::with_capacity((target.len() + extra.len()).min(2048));
    for item in target.iter() {
        seen.insert(normalize_result_path(&item.full_path));
    }

    for item in extra {
        let key = normalize_result_path(&item.full_path);
        if seen.insert(key) {
            target.push(item);
        }
    }
}
