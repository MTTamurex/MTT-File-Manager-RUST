//! Worker thread for global file search via the MTT Search Service.
//! Follows the same Request/Response pattern as file_operation_worker.rs.

use std::collections::HashSet;
use std::sync::mpsc::{Receiver, Sender};

use mtt_search_protocol::{IndexStatusInfo, SearchResultItem};

/// Requests sent from the UI to the global search worker.
pub enum GlobalSearchRequest {
    /// Search for files matching the query.
    Search {
        query: String,
        offset: u32,
        limit: u32,
    },
    /// Check if the search service is available.
    CheckStatus,
}

/// Responses sent from the global search worker to the UI.
pub enum GlobalSearchResponse {
    /// Search results for a given query.
    Results {
        query: String,
        items: Vec<SearchResultItem>,
        offset: u32,
        limit: u32,
        has_more: bool,
    },
    /// Service availability status.
    Status { available: bool, total_indexed: u64 },
    /// Error message.
    Error { query: String, message: String },
}

const OFFLINE_FAILURE_THRESHOLD: u8 = 3;
const STATUS_RETRY_COUNT: usize = 3;
const MIN_QUERY_LEN_FOR_SERVICE_SEARCH: usize = 2;

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

fn append_unique_items(
    target: &mut Vec<SearchResultItem>,
    extra: Vec<SearchResultItem>,
    max_limit: usize,
) {
    if target.len() >= max_limit {
        return;
    }

    let mut seen_paths = HashSet::with_capacity((target.len() + extra.len()).min(2048));
    for item in target.iter() {
        seen_paths.insert(normalize_result_path(&item.full_path));
    }

    for item in extra {
        if target.len() >= max_limit {
            break;
        }

        let key = normalize_result_path(&item.full_path);
        if seen_paths.insert(key) {
            target.push(item);
        }
    }
}

fn query_service_with_retry(
    query: &str,
    offset: u32,
    limit: u32,
) -> Result<crate::infrastructure::global_search::SearchPage, String> {
    match crate::infrastructure::global_search::search(query, offset, limit) {
        Ok(page) => Ok(page),
        Err(e) if e.contains("All pipe instances are busy") => {
            std::thread::sleep(std::time::Duration::from_millis(200));
            crate::infrastructure::global_search::search(query, offset, limit)
        }
        Err(e) => Err(e),
    }
}

fn should_skip_service_query(query: &str, _offset: u32) -> bool {
    query.chars().count() < MIN_QUERY_LEN_FOR_SERVICE_SEARCH
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
                    mut offset,
                    mut limit,
                } => {
                    // Coalesce rapid typing bursts:
                    // process only the latest queued Search before touching IPC.
                    let mut pending_status_check = false;
                    while let Ok(next) = receiver.try_recv() {
                        match next {
                            GlobalSearchRequest::Search {
                                query: next_query,
                                offset: next_offset,
                                limit: next_limit,
                            } => {
                                query = next_query;
                                offset = next_offset;
                                limit = next_limit;
                            }
                            GlobalSearchRequest::CheckStatus => {
                                pending_status_check = true;
                            }
                        }
                    }

                    let max_limit = limit as usize;
                    if query.is_empty() || max_limit == 0 {
                        let _ = sender.send(GlobalSearchResponse::Results {
                            query,
                            items: Vec::new(),
                            offset,
                            limit,
                            has_more: false,
                        });
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
                        continue;
                    }

                    if should_skip_service_query(&query, offset) {
                        let (local_items, local_has_more) =
                            session_index.search_page(&query, offset as usize, max_limit);
                        let _ = sender.send(GlobalSearchResponse::Results {
                            query,
                            items: local_items,
                            offset,
                            limit,
                            has_more: local_has_more,
                        });

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
                        continue;
                    }

                    // Never scan drives in the query path; use cached session index only.
                    // Refresh happens in status cycles to keep typing/search latency stable.
                    session_index.poll_fast_updates();

                    match query_service_with_retry(&query, offset, limit) {
                        Ok(service_page) => {
                            let mut merged = service_page.items;
                            let mut has_more = service_page.has_more;

                            // If service page exhausted before filling this page, continue from the
                            // session-only index using the remaining portion.
                            if !service_page.has_more && merged.len() < max_limit {
                                let service_total = service_page
                                    .total_matches
                                    .unwrap_or(offset.saturating_add(merged.len() as u32));
                                let local_offset = offset.saturating_sub(service_total);
                                let local_limit = max_limit.saturating_sub(merged.len());

                                if local_limit > 0 {
                                    let (local_items, local_has_more) = session_index.search_page(
                                        &query,
                                        local_offset as usize,
                                        local_limit,
                                    );
                                    append_unique_items(&mut merged, local_items, max_limit);
                                    has_more = local_has_more;
                                } else {
                                    has_more = false;
                                }
                            } else if !service_page.has_more && merged.len() == max_limit {
                                // Boundary case: service page ended exactly at service tail.
                                // Check if session-only index has at least one item for the next page.
                                if let Some(service_total) = service_page.total_matches {
                                    let next_offset = offset.saturating_add(merged.len() as u32);
                                    if next_offset >= service_total {
                                        let local_offset =
                                            next_offset.saturating_sub(service_total);
                                        let (probe_items, probe_has_more) = session_index
                                            .search_page(&query, local_offset as usize, 1);
                                        has_more = !probe_items.is_empty() || probe_has_more;
                                    }
                                }
                            }

                            let _ = sender.send(GlobalSearchResponse::Results {
                                query,
                                items: merged,
                                offset,
                                limit,
                                has_more,
                            });
                        }
                        Err(e) => {
                            let (local_items, local_has_more) =
                                session_index.search_page(&query, offset as usize, max_limit);

                            if local_items.is_empty() {
                                let _ =
                                    sender.send(GlobalSearchResponse::Error { query, message: e });
                            } else {
                                log::warn!(
                                    "[GLOBAL-SEARCH] Service query failed, returning session index results: {}",
                                    e
                                );
                                let _ = sender.send(GlobalSearchResponse::Results {
                                    query,
                                    items: local_items,
                                    offset,
                                    limit,
                                    has_more: local_has_more,
                                });
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
