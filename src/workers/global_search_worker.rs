//! Worker thread for global file search via the MTT Search Service.
//! Follows the same Request/Response pattern as file_operation_worker.rs.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use mtt_search_protocol::{IndexStatusInfo, SearchResultItem, VolumeStatus};

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
    /// Enable or disable proactive status polling while the overlay is open.
    SetStatusTracking { active: bool },
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
        total_matches: Option<u32>,
    },
    TotalCount {
        query: String,
        total_matches: u32,
    },
    /// Service availability status.
    Status {
        available: bool,
        total_indexed: u64,
        session_total_indexed: u64,
        indexing_in_progress: bool,
        volumes: Vec<VolumeStatus>,
    },
    /// Error message.
    Error {
        query: String,
        message: String,
    },
}

const OFFLINE_FAILURE_THRESHOLD: u8 = 3;
const STATUS_RETRY_COUNT: usize = 1;
const SEARCH_RETRY_COUNT: usize = 2;
const MIN_QUERY_LEN_FOR_SERVICE_SEARCH: usize = 2;
const TOTAL_COUNT_PAGE_LIMIT: u32 = 1_000;
const ACTIVE_STATUS_POLL_MS: u64 = 250;
const IDLE_STATUS_POLL_MS: u64 = 1_000;

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

fn service_volume_letters(volumes: &[VolumeStatus]) -> HashSet<char> {
    volumes.iter().map(|volume| volume.drive_letter).collect()
}

fn volume_is_actively_scanning(volume: &VolumeStatus) -> bool {
    volume.state == "scanning"
}

fn status_poll_interval(
    tracking_active: bool,
    service_available: bool,
    volumes: &[VolumeStatus],
) -> Option<std::time::Duration> {
    if !tracking_active {
        return None;
    }

    Some(
        if !service_available
            || volumes.is_empty()
            || volumes.iter().any(volume_is_actively_scanning)
        {
            std::time::Duration::from_millis(ACTIVE_STATUS_POLL_MS)
        } else {
            std::time::Duration::from_millis(IDLE_STATUS_POLL_MS)
        },
    )
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
    let mut last_error = None;

    for attempt in 0..SEARCH_RETRY_COUNT {
        match crate::infrastructure::global_search::search(query, offset, limit) {
            Ok(page) => return Ok(page),
            Err(e) => {
                let transient = is_transient_ipc_error(&e);
                last_error = Some(e.clone());

                if !transient || attempt + 1 >= SEARCH_RETRY_COUNT {
                    break;
                }

                let _ = crate::infrastructure::global_search::warm_index();
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
        }
    }

    Err(last_error.unwrap_or_else(|| "Search service query failed".to_string()))
}

fn should_skip_service_query(query: &str, _offset: u32) -> bool {
    query.chars().count() < MIN_QUERY_LEN_FOR_SERVICE_SEARCH
}

fn load_exact_service_total_matches(
    query: &str,
    generation: u64,
    active_generation: &AtomicU64,
) -> Result<Option<u32>, String> {
    let mut offset = 0u32;

    loop {
        if active_generation.load(Ordering::Relaxed) != generation {
            return Ok(None);
        }

        let page = query_service_with_retry(query, offset, TOTAL_COUNT_PAGE_LIMIT)?;
        let page_len = page.items.len() as u32;

        if !page.has_more {
            return Ok(Some(
                page.total_matches
                    .unwrap_or(offset.saturating_add(page_len)),
            ));
        }

        if page_len == 0 {
            return Ok(Some(offset));
        }

        offset = offset.saturating_add(page_len);
    }
}

fn spawn_total_count_task(
    sender: Sender<GlobalSearchResponse>,
    ctx: eframe::egui::Context,
    query: String,
    local_total_matches: u32,
    generation: u64,
    active_generation: Arc<AtomicU64>,
    in_flight: Arc<AtomicBool>,
) {
    // Limit to one in-flight total count task at a time to prevent unbounded
    // thread accumulation during rapid typing.
    if in_flight.swap(true, Ordering::AcqRel) {
        return; // Another task is already running — skip this one.
    }

    let in_flight_for_task = Arc::clone(&in_flight);
    let spawn_result = crate::spawn_named("global-search-total-count", move || {
        // RAII guard to reset in_flight on any exit path (including panic).
        struct InFlightGuard(Arc<AtomicBool>);
        impl Drop for InFlightGuard {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _guard = InFlightGuard(in_flight_for_task);

        match load_exact_service_total_matches(&query, generation, &active_generation) {
            Ok(Some(service_total_matches)) => {
                if active_generation.load(Ordering::Relaxed) != generation {
                    return;
                }

                let _ = sender.send(GlobalSearchResponse::TotalCount {
                    query,
                    total_matches: service_total_matches.saturating_add(local_total_matches),
                });
                ctx.request_repaint();
            }
            Ok(None) => {}
            Err(error) => {
                if active_generation.load(Ordering::Relaxed) == generation {
                    log::debug!(
                        "[GLOBAL-SEARCH] Exact total count unavailable for '{}': {}",
                        query,
                        error
                    );
                }
            }
        }
    });

    if let Err(error) = spawn_result {
        in_flight.store(false, Ordering::Release);
        log::error!(
            "[GLOBAL-SEARCH] Failed to spawn total count task: {}",
            error
        );
    }
}

fn refresh_and_send_status(
    sender: &Sender<GlobalSearchResponse>,
    session_index: &mut crate::infrastructure::user_session_search::UserSessionSearchIndex,
    last_known_available: &mut bool,
    last_known_total_indexed: &mut u64,
    last_known_status_volumes: &mut Vec<VolumeStatus>,
    last_known_service_executable_path: &mut String,
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
            if *last_known_service_executable_path != status.service_executable_path {
                log::info!(
                    "[GLOBAL-SEARCH] Connected to service binary: {}",
                    status.service_executable_path
                );
                *last_known_service_executable_path = status.service_executable_path.clone();
            }
            *last_known_available = true;
            *last_known_total_indexed = status.total_files_indexed;
            *last_known_status_volumes = status.volumes.clone();
            *consecutive_failures = 0;
        } else {
            *consecutive_failures = (*consecutive_failures).saturating_add(1);
            let transient = last_error.as_deref().is_some_and(is_transient_ipc_error);
            if !(transient && *last_known_available)
                && *consecutive_failures >= OFFLINE_FAILURE_THRESHOLD
            {
                *last_known_available = false;
            }
        }
    } else {
        *consecutive_failures = (*consecutive_failures).saturating_add(1);
        if *consecutive_failures >= OFFLINE_FAILURE_THRESHOLD {
            *last_known_available = false;
        }
    }

    let service_volume_letters = service_volume_letters(last_known_status_volumes);
    session_index.refresh(&service_volume_letters, *last_known_available, false);
    let local_total = session_index.total_indexed();
    let available = *last_known_available || session_index.has_indexed_items();
    let total_indexed = *last_known_total_indexed;
    let indexing_in_progress = last_known_status_volumes
        .iter()
        .any(volume_is_actively_scanning);

    let _ = sender.send(GlobalSearchResponse::Status {
        available,
        total_indexed,
        session_total_indexed: local_total,
        indexing_in_progress,
        volumes: last_known_status_volumes.clone(),
    });
}

/// Starts the global search worker thread.
pub fn start_global_search_worker(
    receiver: Receiver<GlobalSearchRequest>,
    sender: Sender<GlobalSearchResponse>,
    ctx: eframe::egui::Context,
) {
    std::thread::spawn(move || {
        let total_count_generation = Arc::new(AtomicU64::new(0));
        let total_count_in_flight = Arc::new(AtomicBool::new(false));
        let mut last_known_available = false;
        let mut last_known_total_indexed = 0u64;
        let mut last_known_status_volumes = Vec::<VolumeStatus>::new();
        let mut last_known_service_executable_path = String::new();
        let mut consecutive_failures = 0u8;
        let mut status_tracking_active = false;
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
            &mut last_known_status_volumes,
            &mut last_known_service_executable_path,
            &mut consecutive_failures,
        );
        ctx.request_repaint();

        loop {
            let initial_request = match status_poll_interval(
                status_tracking_active,
                last_known_available,
                &last_known_status_volumes,
            ) {
                Some(timeout) => match receiver.recv_timeout(timeout) {
                    Ok(request) => Some(request),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                },
                None => match receiver.recv() {
                    Ok(request) => Some(request),
                    Err(_) => break,
                },
            };

            // ── Coalesce: drain the channel, keeping only the latest Search
            //    and noting whether any CheckStatus was enqueued. ──────────
            let mut latest_search: Option<(String, u32, u32)> = None;
            let mut pending_status_check = initial_request.is_none();
            let mut next_tracking_state: Option<bool> = None;

            if let Some(request) = initial_request {
                match request {
                    GlobalSearchRequest::Search {
                        query,
                        offset,
                        limit,
                    } => {
                        latest_search = Some((query, offset, limit));
                    }
                    GlobalSearchRequest::CheckStatus => {
                        pending_status_check = true;
                    }
                    GlobalSearchRequest::SetStatusTracking { active } => {
                        next_tracking_state = Some(active);
                    }
                }
            }

            while let Ok(next) = receiver.try_recv() {
                match next {
                    GlobalSearchRequest::Search {
                        query,
                        offset,
                        limit,
                    } => {
                        latest_search = Some((query, offset, limit));
                    }
                    GlobalSearchRequest::CheckStatus => {
                        pending_status_check = true;
                    }
                    GlobalSearchRequest::SetStatusTracking { active } => {
                        next_tracking_state = Some(active);
                    }
                }
            }

            if let Some(active) = next_tracking_state {
                status_tracking_active = active;
                if active {
                    pending_status_check = true;
                }
            }

            // ── Search always takes priority over status checks ──────────
            if let Some((query, offset, limit)) = latest_search {
                let total_count_token = if offset == 0 {
                    total_count_generation.fetch_add(1, Ordering::Relaxed) + 1
                } else {
                    total_count_generation.load(Ordering::Relaxed)
                };
                let max_limit = limit as usize;
                if query.is_empty() || max_limit == 0 {
                    let _ = sender.send(GlobalSearchResponse::Results {
                        query,
                        items: Vec::new(),
                        offset,
                        limit,
                        has_more: false,
                        total_matches: Some(0),
                    });
                    // Don't cascade into a status check — let the periodic timer handle it.
                    ctx.request_repaint();
                    continue;
                }

                if should_skip_service_query(&query, offset) {
                    let local_total_matches = session_index.count_matches(&query);
                    let (local_items, local_has_more) =
                        session_index.search_page(&query, offset as usize, max_limit);
                    let _ = sender.send(GlobalSearchResponse::Results {
                        query,
                        items: local_items,
                        offset,
                        limit,
                        has_more: local_has_more,
                        total_matches: Some(local_total_matches),
                    });
                    ctx.request_repaint();
                    continue;
                }

                // Never scan drives in the query path; use cached session index only.
                session_index.poll_fast_updates();

                match query_service_with_retry(&query, offset, limit) {
                    Ok(service_page) => {
                        let local_total_matches =
                            if offset == 0 || service_page.total_matches.is_some() {
                                Some(session_index.count_matches(&query))
                            } else {
                                None
                            };
                        let mut merged = service_page.items;
                        let mut has_more = service_page.has_more;
                        let total_matches = service_page.total_matches.map(|service_total| {
                            service_total.saturating_add(local_total_matches.unwrap_or(0))
                        });

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
                            if let Some(service_total) = service_page.total_matches {
                                let next_offset = offset.saturating_add(merged.len() as u32);
                                if next_offset >= service_total {
                                    let local_offset = next_offset.saturating_sub(service_total);
                                    let (probe_items, probe_has_more) =
                                        session_index.search_page(&query, local_offset as usize, 1);
                                    has_more = !probe_items.is_empty() || probe_has_more;
                                }
                            }
                        }

                        let _ = sender.send(GlobalSearchResponse::Results {
                            query: query.clone(),
                            items: merged,
                            offset,
                            limit,
                            has_more,
                            total_matches,
                        });

                        if offset == 0 && total_matches.is_none() {
                            spawn_total_count_task(
                                sender.clone(),
                                ctx.clone(),
                                query,
                                local_total_matches.unwrap_or(0),
                                total_count_token,
                                Arc::clone(&total_count_generation),
                                Arc::clone(&total_count_in_flight),
                            );
                        }
                    }
                    Err(e) => {
                        let transient = is_transient_ipc_error(&e);
                        if transient {
                            let _ = crate::infrastructure::global_search::warm_index();
                            refresh_and_send_status(
                                &sender,
                                &mut session_index,
                                &mut last_known_available,
                                &mut last_known_total_indexed,
                                &mut last_known_status_volumes,
                                &mut last_known_service_executable_path,
                                &mut consecutive_failures,
                            );
                        }

                        let local_total_matches = session_index.count_matches(&query);
                        let (local_items, local_has_more) =
                            session_index.search_page(&query, offset as usize, max_limit);
                        let can_use_local_only = !last_known_available
                            && last_known_total_indexed == 0
                            && !local_items.is_empty();

                        if can_use_local_only {
                            log::warn!(
                                "[GLOBAL-SEARCH] Service unavailable, using session-only fallback: {}",
                                e
                            );
                            let _ = sender.send(GlobalSearchResponse::Results {
                                query,
                                items: local_items,
                                offset,
                                limit,
                                has_more: local_has_more,
                                total_matches: Some(local_total_matches),
                            });
                        } else {
                            let _ = sender.send(GlobalSearchResponse::Error { query, message: e });
                        }
                    }
                }
                // Don't cascade status check after search — keeps the worker responsive.
            } else if pending_status_check {
                // Only process status check when no search is pending.
                refresh_and_send_status(
                    &sender,
                    &mut session_index,
                    &mut last_known_available,
                    &mut last_known_total_indexed,
                    &mut last_known_status_volumes,
                    &mut last_known_service_executable_path,
                    &mut consecutive_failures,
                );
            }

            ctx.request_repaint();
        }
    });
}
