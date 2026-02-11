//! Worker thread for global file search via the MTT Search Service.
//! Follows the same Request/Response pattern as file_operation_worker.rs.

use std::sync::mpsc::{Receiver, Sender};

use mtt_search_protocol::SearchResultItem;

/// Requests sent from the UI to the global search worker.
pub enum GlobalSearchRequest {
    /// Search for files matching the query.
    Search { query: String, max_results: u32 },
    /// Check if the search service is available.
    CheckStatus,
}

/// Responses sent from the global search worker to the UI.
pub enum GlobalSearchResponse {
    /// Search results for a given query.
    Results {
        query: String,
        items: Vec<SearchResultItem>,
    },
    /// Service availability status.
    Status { available: bool, total_indexed: u64 },
    /// Error message.
    Error { query: String, message: String },
}

/// Starts the global search worker thread.
pub fn start_global_search_worker(
    receiver: Receiver<GlobalSearchRequest>,
    sender: Sender<GlobalSearchResponse>,
    ctx: eframe::egui::Context,
) {
    std::thread::spawn(move || {
        let send_status = |sender: &Sender<GlobalSearchResponse>| {
            let available = crate::infrastructure::global_search::ping();
            let total = if available {
                crate::infrastructure::global_search::get_status()
                    .map(|s| s.total_files_indexed)
                    .unwrap_or(0)
            } else {
                0
            };
            let _ = sender.send(GlobalSearchResponse::Status {
                available,
                total_indexed: total,
            });
        };

        while let Ok(request) = receiver.recv() {
            match request {
                GlobalSearchRequest::Search {
                    mut query,
                    mut max_results,
                } => {
                    // Coalesce rapid typing bursts:
                    // process only the latest queued Search before touching IPC.
                    let mut pending_status_check = false;
                    while let Ok(next) = receiver.try_recv() {
                        match next {
                            GlobalSearchRequest::Search {
                                query: next_query,
                                max_results: next_max_results,
                            } => {
                                query = next_query;
                                max_results = next_max_results;
                            }
                            GlobalSearchRequest::CheckStatus => {
                                pending_status_check = true;
                            }
                        }
                    }

                    match crate::infrastructure::global_search::search(&query, max_results) {
                        Ok(items) => {
                            let _ = sender.send(GlobalSearchResponse::Results { query, items });
                        }
                        Err(e) => {
                            let _ = sender.send(GlobalSearchResponse::Error { query, message: e });
                        }
                    }

                    if pending_status_check {
                        send_status(&sender);
                    }
                }
                GlobalSearchRequest::CheckStatus => {
                    send_status(&sender);
                }
            }
            ctx.request_repaint();
        }
    });
}
