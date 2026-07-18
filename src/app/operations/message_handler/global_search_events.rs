//! Processes responses from the global search worker.

use crate::app::state::ImageViewerApp;
use crate::workers::global_search_worker::GlobalSearchResponse;
use crate::workers::tagged_results_worker::{normalize_search_path_key, TaggedResultsResponse};
use eframe::egui;
use std::time::{Duration, Instant};

impl ImageViewerApp {
    pub(super) fn process_global_search_events(&mut self) {
        const MAX_GLOBAL_SEARCH_MSGS_PER_FRAME: usize = 48;
        let budget = if self.frame_time_peak_ms > 33.33 {
            Duration::from_millis(1)
        } else if self.frame_time_peak_ms > 25.0 {
            Duration::from_millis(2)
        } else {
            Duration::from_millis(4)
        };

        let start = Instant::now();
        let mut processed = 0usize;
        let mut has_more = false;

        while processed < MAX_GLOBAL_SEARCH_MSGS_PER_FRAME {
            if start.elapsed() >= budget {
                has_more = true;
                break;
            }

            let response = match self.global_search.receiver.try_recv() {
                Ok(response) => response,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            };
            processed += 1;

            match response {
                GlobalSearchResponse::Results {
                    query,
                    items,
                    offset,
                    limit,
                    has_more,
                    total_matches,
                } => {
                    // Only apply if the query still matches (user may have typed more)
                    if query == self.global_search.query {
                        if offset == 0 {
                            self.global_search.cancel_tagged_results_request();
                            self.global_search.service_results_loaded = items.len() as u32;
                            self.global_search.results = items;
                            self.global_search.service_results_prefix_len =
                                self.global_search.results.len();
                            self.global_search.clear_result_selection();
                            self.global_search.results_generation += 1;
                            self.global_search.reset_sort_metadata_for_current_results();
                            self.global_search.total_matches = total_matches.map(u64::from);
                        } else if offset == self.global_search.service_results_loaded {
                            let raw_items_len = items.len() as u32;
                            self.global_search.truncate_tagged_results();
                            self.global_search.tagged_results_cache_key = None;
                            append_unique_results(&mut self.global_search.results, items);
                            self.global_search.service_results_loaded =
                                offset.saturating_add(raw_items_len);
                            self.global_search.service_results_prefix_len =
                                self.global_search.results.len();
                            self.global_search.results_generation += 1;
                            if let Some(total_matches) = total_matches {
                                self.global_search.total_matches = Some(u64::from(total_matches));
                            }
                        } else {
                            // Stale page response (offset mismatch), ignore.
                            continue;
                        }

                        self.global_search.loading = false;
                        self.global_search.in_flight_query = None;
                        self.global_search.in_flight_started_at = None;
                        self.global_search.requested_offset = offset;
                        self.global_search.requested_limit = limit;
                        self.global_search.has_more_results = has_more;
                    }
                }
                GlobalSearchResponse::TotalCount {
                    query,
                    total_matches,
                } => {
                    if query == self.global_search.query {
                        self.global_search.total_matches = Some(u64::from(total_matches));
                    }
                }
                GlobalSearchResponse::Status {
                    available,
                    total_indexed,
                    session_total_indexed,
                    indexing_in_progress,
                    volumes,
                } => {
                    let now = Instant::now();
                    let progress_changed = total_indexed != self.global_search.total_indexed
                        || indexing_in_progress != self.global_search.indexing_in_progress
                        || status_volumes_changed(&self.global_search.status_volumes, &volumes);

                    self.global_search.available = available;
                    self.global_search.total_indexed = total_indexed;
                    self.global_search.session_total_indexed = session_total_indexed;
                    self.global_search.indexing_in_progress = indexing_in_progress;
                    self.global_search.status_volumes = volumes;
                    self.global_search.last_status_received_at = now;
                    if progress_changed {
                        self.global_search.last_progress_advance_at = now;
                    }
                }
                GlobalSearchResponse::Error { query, message } => {
                    if query == self.global_search.query {
                        self.global_search.loading = false;
                        self.global_search.in_flight_query = None;
                        self.global_search.in_flight_started_at = None;
                        self.global_search.has_more_results = false;
                        self.global_search.total_matches = None;
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

        if processed >= MAX_GLOBAL_SEARCH_MSGS_PER_FRAME {
            has_more = true;
        }

        if has_more {
            self.ui_ctx.request_repaint();
        }

        drain_tagged_results_responses(self);

        // Drain tooltip worker responses (P0-02/P0-03)
        drain_tooltip_responses(self);

        if self.global_search.active {
            self.ui_ctx
                .request_repaint_after(if self.global_search.indexing_in_progress {
                    Duration::from_millis(200)
                } else {
                    Duration::from_millis(500)
                });
            return;
        }

        // Check availability at a moderate interval. Avoid aggressive polling
        // that can starve the single-threaded worker when it should be processing
        // search requests.
        let interval = if self.global_search.available {
            Duration::from_secs(30)
        } else {
            Duration::from_secs(5)
        };

        if self.global_search.last_check.elapsed() > interval {
            self.global_search.last_check = Instant::now();
            let _ = self
                .global_search
                .sender
                .send(crate::workers::global_search_worker::GlobalSearchRequest::CheckStatus);
        }
    }
}

fn status_volumes_changed(
    current: &[mtt_search_protocol::VolumeStatus],
    next: &[mtt_search_protocol::VolumeStatus],
) -> bool {
    if current.len() != next.len() {
        return true;
    }

    current.iter().zip(next.iter()).any(|(left, right)| {
        left.drive_letter != right.drive_letter
            || left.state != right.state
            || left.files_indexed != right.files_indexed
            || left.phase != right.phase
            || left.phase_progress != right.phase_progress
            || left.phase_total != right.phase_total
    })
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

fn append_unique_results(
    target: &mut Vec<mtt_search_protocol::SearchResultItem>,
    extra: Vec<mtt_search_protocol::SearchResultItem>,
) {
    if extra.is_empty() {
        return;
    }

    let mut seen = std::collections::HashSet::with_capacity((target.len() + extra.len()).min(2048));
    for item in target.iter() {
        seen.insert(normalize_search_path_key(&item.full_path));
    }

    for item in extra {
        let key = normalize_search_path_key(&item.full_path);
        if seen.insert(key) {
            target.push(item);
        }
    }
}

fn drain_tagged_results_responses(app: &mut ImageViewerApp) {
    while let Ok(response) = app.global_search.tagged_results_receiver.try_recv() {
        apply_tagged_results_response(app, response);
    }
}

fn apply_tagged_results_response(app: &mut ImageViewerApp, response: TaggedResultsResponse) {
    let expected = app
        .global_search
        .tagged_results_inflight
        .as_ref()
        .is_some_and(|(request_id, cache_key)| {
            *request_id == response.request_id && cache_key == &response.cache_key
        });
    if !expected {
        return;
    }
    app.global_search.tagged_results_inflight = None;

    let current_key = (
        app.global_search.query.clone(),
        app.global_search.tag_filter.clone(),
        app.tag_assignments_epoch,
    );
    if current_key != response.cache_key
        || matches!(
            current_key.1,
            crate::app::global_search_state::GlobalSearchTagFilter::All
        )
    {
        return;
    }

    app.global_search.truncate_tagged_results();
    let previous_len = app.global_search.results.len();
    let response_len = response.items.len();
    append_unique_results(&mut app.global_search.results, response.items);
    let added_len = app.global_search.results.len() - previous_len;
    app.global_search.tagged_results_cache_key =
        if tagged_response_needs_retry(response.limit_reached, response_len, added_len) {
            None
        } else {
            Some(response.cache_key)
        };
    if added_len > 0 {
        app.global_search.reset_sort_metadata_for_current_results();
        app.global_search.cached_filtered_indices.clear();
        app.global_search.cached_sorted_indices.clear();
        app.global_search.results_generation = app.global_search.results_generation.wrapping_add(1);
    }
}

fn tagged_response_needs_retry(limit_reached: bool, response_len: usize, added_len: usize) -> bool {
    limit_reached && added_len < response_len
}

/// Drains responses from the tooltip background worker, updating caches
/// and clearing inflight markers.
fn drain_tooltip_responses(app: &mut ImageViewerApp) {
    const MAX_TOOLTIP_MSGS_PER_FRAME: usize = 16;
    for _ in 0..MAX_TOOLTIP_MSGS_PER_FRAME {
        match app.global_search.tooltip_receiver.try_recv() {
            Ok(crate::app::global_search_state::TooltipResponse::Metadata {
                path,
                size,
                modified_ts,
                created_ts,
            }) => {
                let was_tooltip_request = app.global_search.tooltip_metadata_inflight.remove(&path);
                app.global_search.apply_sort_metadata(&path, modified_ts);
                app.global_search.apply_created_metadata(&path, created_ts);

                if was_tooltip_request {
                    app.global_search
                        .metadata_cache
                        .put(path.clone(), modified_ts);
                    if let Some(len) = size {
                        app.global_search.size_cache.put(path, Some(len));
                    }
                }
            }
            Ok(crate::app::global_search_state::TooltipResponse::Thumbnail {
                path,
                rgba,
                width,
                height,
            }) => {
                app.global_search.tooltip_thumbnail_inflight.remove(&path);
                // Validate the buffer before it reaches ColorImage::from_rgba_*.
                if !crate::domain::thumbnail::is_valid_rgba_buffer(
                    width,
                    height,
                    crate::domain::thumbnail::MAX_THUMBNAIL_SIDE,
                    rgba.len(),
                ) {
                    log::warn!(
                        "[GLOBAL-SEARCH] Rejected invalid tooltip RGBA for {} (w={} h={} len={})",
                        path,
                        width,
                        height,
                        rgba.len()
                    );
                    continue;
                }
                let tex = app.ui_ctx.load_texture(
                    format!("gs_thumb_{}", path),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba,
                    ),
                    egui::TextureOptions::LINEAR,
                );
                app.global_search.tooltip_texture_cache.put(path, tex);
            }
            Ok(crate::app::global_search_state::TooltipResponse::ThumbnailFailed { path }) => {
                app.global_search.tooltip_thumbnail_inflight.remove(&path);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => break,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tagged_response_needs_retry;

    #[test]
    fn retries_limited_tagged_response_when_new_service_page_deduplicates_items() {
        assert!(tagged_response_needs_retry(true, 2_000, 1_750));
        assert!(!tagged_response_needs_retry(true, 2_000, 2_000));
        assert!(!tagged_response_needs_retry(false, 150, 100));
    }
}
