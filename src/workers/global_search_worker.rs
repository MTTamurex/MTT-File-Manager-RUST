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
    Status {
        available: bool,
        total_indexed: u64,
    },
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
        while let Ok(request) = receiver.recv() {
            match request {
                GlobalSearchRequest::Search { query, max_results } => {
                    match crate::infrastructure::global_search::search(&query, max_results) {
                        Ok(items) => {
                            let _ = sender.send(GlobalSearchResponse::Results {
                                query,
                                items,
                            });
                        }
                        Err(e) => {
                            let _ = sender.send(GlobalSearchResponse::Error {
                                query,
                                message: e,
                            });
                        }
                    }
                }
                GlobalSearchRequest::CheckStatus => {
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
                }
            }
            ctx.request_repaint();
        }
    });
}
