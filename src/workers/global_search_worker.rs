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
        const OFFLINE_FAILURE_THRESHOLD: u8 = 3;
        const STATUS_RETRY_COUNT: usize = 3;

        let mut last_known_available = false;
        let mut last_known_total_indexed = 0u64;
        let mut consecutive_failures = 0u8;

        fn is_transient_ipc_error(message: &str) -> bool {
            let m = message.to_ascii_lowercase();
            m.contains("all pipe instances are busy")
                || m.contains("no process is on the other end of the pipe")
                || m.contains("pipe closed during read")
                || m.contains("search service timeout")
                || m.contains("peeknamedpipe failed")
                || m.contains("readfile failed")
                || m.contains("writefile failed")
        }

        let mut send_status = |sender: &Sender<GlobalSearchResponse>| {
            let ping_ok = crate::infrastructure::global_search::ping();
            if ping_ok {
                let mut status_ok = None;
                let mut last_error: Option<String> = None;

                for attempt in 0..STATUS_RETRY_COUNT {
                    match crate::infrastructure::global_search::get_status() {
                        Ok(s) => {
                            status_ok = Some(s.total_files_indexed);
                            break;
                        }
                        Err(e) => {
                            last_error = Some(e.clone());
                            if attempt + 1 < STATUS_RETRY_COUNT && is_transient_ipc_error(&e) {
                                std::thread::sleep(std::time::Duration::from_millis(140));
                                continue;
                            }
                            break;
                        }
                    }
                }

                if let Some(total_indexed) = status_ok {
                    last_known_available = true;
                    last_known_total_indexed = total_indexed;
                    consecutive_failures = 0;
                } else {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                    let transient = last_error.as_deref().is_some_and(is_transient_ipc_error);
                    if !(transient && last_known_available)
                        && consecutive_failures >= OFFLINE_FAILURE_THRESHOLD
                    {
                        last_known_available = false;
                        last_known_total_indexed = 0;
                    }
                }
            } else {
                consecutive_failures = consecutive_failures.saturating_add(1);
                if consecutive_failures >= OFFLINE_FAILURE_THRESHOLD {
                    last_known_available = false;
                    last_known_total_indexed = 0;
                }
            }

            let _ = sender.send(GlobalSearchResponse::Status {
                available: last_known_available,
                total_indexed: last_known_total_indexed,
            });
        };

        // Prime status push at worker startup.
        send_status(&sender);
        ctx.request_repaint();

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
                            if e.contains("All pipe instances are busy") {
                                std::thread::sleep(std::time::Duration::from_millis(200));
                                match crate::infrastructure::global_search::search(
                                    &query,
                                    max_results,
                                ) {
                                    Ok(items) => {
                                        let _ = sender
                                            .send(GlobalSearchResponse::Results { query, items });
                                        if pending_status_check {
                                            send_status(&sender);
                                        }
                                        continue;
                                    }
                                    Err(e2) => {
                                        let _ = sender.send(GlobalSearchResponse::Error {
                                            query,
                                            message: e2,
                                        });
                                    }
                                }
                            } else {
                                let _ =
                                    sender.send(GlobalSearchResponse::Error { query, message: e });
                            }
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
