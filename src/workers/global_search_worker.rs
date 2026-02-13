//! Worker thread for global file search via the MTT Search Service.
//! Follows the same Request/Response pattern as file_operation_worker.rs.

use std::collections::HashSet;
use std::sync::mpsc::{Receiver, Sender};

use mtt_search_protocol::{IndexStatusInfo, SearchResultItem};

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

const OFFLINE_FAILURE_THRESHOLD: u8 = 3;
const STATUS_RETRY_COUNT: usize = 3;

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

fn normalize_result_path(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    let stripped = lower.strip_prefix(r"\\?\").unwrap_or(&lower);

    if stripped.len() > 3 {
        stripped.trim_end_matches('\\').to_string()
    } else {
        stripped.to_string()
    }
}

fn merge_results(
    service_items: Vec<SearchResultItem>,
    local_items: Vec<SearchResultItem>,
    max_results: usize,
) -> Vec<SearchResultItem> {
    let mut merged = Vec::with_capacity(max_results.min(256));
    let mut seen_paths =
        HashSet::with_capacity((service_items.len() + local_items.len()).min(2048));

    for item in service_items.into_iter().chain(local_items) {
        let key = normalize_result_path(&item.full_path);
        if seen_paths.insert(key) {
            merged.push(item);
            if merged.len() >= max_results {
                break;
            }
        }
    }

    merged
}

fn query_service_with_retry(
    query: &str,
    max_results: u32,
) -> Result<Vec<SearchResultItem>, String> {
    match crate::infrastructure::global_search::search(query, max_results) {
        Ok(items) => Ok(items),
        Err(e) if e.contains("All pipe instances are busy") => {
            std::thread::sleep(std::time::Duration::from_millis(200));
            crate::infrastructure::global_search::search(query, max_results)
        }
        Err(e) => Err(e),
    }
}

fn refresh_and_send_status(
    sender: &Sender<GlobalSearchResponse>,
    session_index: &mut crate::infrastructure::user_session_search::UserSessionSearchIndex,
    last_known_available: &mut bool,
    last_known_total_indexed: &mut u64,
    last_known_service_volumes: &mut HashSet<char>,
    consecutive_failures: &mut u8,
) {
    let ping_ok = crate::infrastructure::global_search::ping();
    let mut status_ok: Option<IndexStatusInfo> = None;
    let mut last_error: Option<String> = None;

    if ping_ok {
        for attempt in 0..STATUS_RETRY_COUNT {
            match crate::infrastructure::global_search::get_status() {
                Ok(status) => {
                    status_ok = Some(status);
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

        if let Some(status) = status_ok {
            *last_known_available = true;
            *last_known_total_indexed = status.total_files_indexed;
            *last_known_service_volumes = status.volumes.iter().map(|v| v.drive_letter).collect();
            *consecutive_failures = 0;
        } else {
            *consecutive_failures = (*consecutive_failures).saturating_add(1);
            let transient = last_error.as_deref().is_some_and(is_transient_ipc_error);
            if !(transient && *last_known_available)
                && *consecutive_failures >= OFFLINE_FAILURE_THRESHOLD
            {
                *last_known_available = false;
                *last_known_total_indexed = 0;
            }
        }
    } else {
        *consecutive_failures = (*consecutive_failures).saturating_add(1);
        if *consecutive_failures >= OFFLINE_FAILURE_THRESHOLD {
            *last_known_available = false;
            *last_known_total_indexed = 0;
        }
    }

    session_index.refresh(last_known_service_volumes, *last_known_available, false);
    let local_total = session_index.total_indexed();
    let available = *last_known_available || session_index.has_indexed_items();
    let total_indexed = (*last_known_total_indexed).saturating_add(local_total);

    let _ = sender.send(GlobalSearchResponse::Status {
        available,
        total_indexed,
    });
}

/// Starts the global search worker thread.
pub fn start_global_search_worker(
    receiver: Receiver<GlobalSearchRequest>,
    sender: Sender<GlobalSearchResponse>,
    ctx: eframe::egui::Context,
) {
    std::thread::spawn(move || {
        let mut last_known_available = false;
        let mut last_known_total_indexed = 0u64;
        let mut last_known_service_volumes = HashSet::<char>::new();
        let mut consecutive_failures = 0u8;
        let mut session_index =
            crate::infrastructure::user_session_search::UserSessionSearchIndex::new();

        // Warm the service's in-memory index so paged-out memory is brought back to RAM
        // before the user opens global search.
        let _ = crate::infrastructure::global_search::warm_index();

        // Prime status push at worker startup.
        refresh_and_send_status(
            &sender,
            &mut session_index,
            &mut last_known_available,
            &mut last_known_total_indexed,
            &mut last_known_service_volumes,
            &mut consecutive_failures,
        );
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

                    session_index.refresh(&last_known_service_volumes, last_known_available, true);
                    let local_items = session_index.search(&query, max_results as usize);

                    match query_service_with_retry(&query, max_results) {
                        Ok(service_items) => {
                            let items =
                                merge_results(service_items, local_items, max_results as usize);
                            let _ = sender.send(GlobalSearchResponse::Results { query, items });
                        }
                        Err(e) => {
                            if local_items.is_empty() {
                                let _ =
                                    sender.send(GlobalSearchResponse::Error { query, message: e });
                            } else {
                                log::warn!(
                                    "[GLOBAL-SEARCH] Service query failed, returning session index results: {}",
                                    e
                                );
                                let items =
                                    merge_results(Vec::new(), local_items, max_results as usize);
                                let _ = sender.send(GlobalSearchResponse::Results { query, items });
                            }
                        }
                    }

                    if pending_status_check {
                        refresh_and_send_status(
                            &sender,
                            &mut session_index,
                            &mut last_known_available,
                            &mut last_known_total_indexed,
                            &mut last_known_service_volumes,
                            &mut consecutive_failures,
                        );
                    }
                }
                GlobalSearchRequest::CheckStatus => {
                    refresh_and_send_status(
                        &sender,
                        &mut session_index,
                        &mut last_known_available,
                        &mut last_known_total_indexed,
                        &mut last_known_service_volumes,
                        &mut consecutive_failures,
                    );
                }
            }
            ctx.request_repaint();
        }
    });
}
