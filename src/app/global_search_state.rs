use eframe::egui;
use lru::LruCache;
use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

const MAX_SORT_METADATA_IN_FLIGHT: usize = 32;

// --- Tooltip background worker types ---

/// Request sent to the tooltip background worker.
pub enum TooltipRequest {
    /// Read fs::metadata for size + modified timestamp.
    Metadata(String),
    /// Read disk cache and decode WebP to RGBA.
    Thumbnail(String),
}

/// Response from the tooltip background worker.
pub enum TooltipResponse {
    Metadata {
        path: String,
        size: Option<u64>,
        modified_ts: u64,
        created_ts: Option<u64>,
    },
    Thumbnail {
        path: String,
        rgba: Vec<u8>,
        width: u32,
        height: u32,
    },
    ThumbnailFailed {
        path: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalSearchCategory {
    All,
    Files,
    Folders,
    Images,
    Videos,
    Audio,
    Documents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalSearchSortMode {
    Relevance,
    ModifiedDate,
    Name,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum GlobalSearchTagFilter {
    /// No tag filter applied — show all results, regardless of whether they have tags.
    #[default]
    All,
    /// Show only results that have at least one tag assigned (any tag).
    Any,
    /// Show only results that have at least one of the specified tag IDs (OR semantics).
    Selected(Vec<i64>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CreatedMetadataState {
    Pending,
    Unavailable,
    Available(u64),
}

/// Combined filter parameters used for cache invalidation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalSearchFilters {
    pub category: GlobalSearchCategory,
    pub drive_filter: Option<char>,
    pub min_size_mb: Option<u64>,
    pub max_size_mb: Option<u64>,
    pub created_after: Option<u64>,
    pub created_before: Option<u64>,
    pub tag_filter: GlobalSearchTagFilter,
}

pub struct GlobalSearchState {
    pub sender: Sender<crate::workers::global_search_worker::GlobalSearchRequest>,
    pub receiver: Receiver<crate::workers::global_search_worker::GlobalSearchResponse>,
    pub query: String,
    pub results: Vec<mtt_search_protocol::SearchResultItem>,
    pub selected_index: Option<usize>,
    pub focus_request: bool,
    pub size_cache: LruCache<String, Option<u64>>,
    /// Bounded cache for tooltip thumbnail textures (prevents VRAM leak).
    pub tooltip_texture_cache: LruCache<String, egui::TextureHandle>,
    /// Bounded cache for tooltip modified-date timestamps (avoids per-frame fs::metadata).
    pub metadata_cache: LruCache<String, u64>,
    pub category: GlobalSearchCategory,
    pub drive_filter: Option<char>,
    pub sort_mode: GlobalSearchSortMode,
    pub sort_descending: bool,

    // --- Advanced filter fields ---
    pub min_size_mb: Option<u64>,
    pub max_size_mb: Option<u64>,
    pub created_after: Option<u64>,
    pub created_before: Option<u64>,
    /// Tag filter (see `GlobalSearchTagFilter`).
    pub tag_filter: GlobalSearchTagFilter,
    /// Created After date components (0 = not set).
    pub created_after_month: u32,
    pub created_after_day: u32,
    pub created_after_year: u32,
    /// Created After text buffers for input fields.
    pub created_after_month_text: String,
    pub created_after_day_text: String,
    pub created_after_year_text: String,
    /// Created Before date components (0 = not set).
    pub created_before_month: u32,
    pub created_before_day: u32,
    pub created_before_year: u32,
    /// Created Before text buffers for input fields.
    pub created_before_month_text: String,
    pub created_before_day_text: String,
    pub created_before_year_text: String,
    pub active: bool,
    pub opened_at: Instant,
    pub loading: bool,
    pub pending_query_dispatch_at: Option<Instant>,
    pub in_flight_query: Option<String>,
    pub in_flight_started_at: Option<Instant>,
    pub requested_offset: u32,
    pub requested_limit: u32,
    pub has_more_results: bool,
    pub available: bool,
    pub last_check: Instant,
    pub last_status_received_at: Instant,
    pub last_progress_advance_at: Instant,
    pub total_indexed: u64,
    pub session_total_indexed: u64,
    pub total_matches: Option<u64>,
    pub status_volumes: Vec<mtt_search_protocol::VolumeStatus>,
    pub indexing_in_progress: bool,
    /// Manual scroll offset for virtualized results list.
    pub scroll_offset_y: f32,
    /// Previous target scroll offset used to detect active scroll updates.
    pub last_scroll_offset_y: f32,
    /// Tracks last scroll change for adaptive overscan.
    pub last_scroll_time: Instant,

    // --- Cached filter state (avoids O(N) recomputation every frame) ---
    /// Monotonic counter incremented whenever `results` changes.
    pub results_generation: u64,
    /// Cached `build_filtered_indices` output.
    pub cached_filtered_indices: Vec<usize>,
    /// Cached `available_drives` output.
    pub cached_available_drives: Vec<char>,
    /// Generation + filter params when the cache was last built.
    filter_cache_key: (u64, GlobalSearchFilters, u64, u64),

    // --- Cached sorted indices (includes filter + sort) ---
    pub cached_sorted_indices: Vec<usize>,
    sort_cache_key: (
        u64,
        GlobalSearchFilters,
        GlobalSearchSortMode,
        bool,
        u64,
        u64,
        u64,
    ),
    /// Incremented whenever new metadata is loaded for sorting; forces a re-sort next frame.
    sort_metadata_epoch: u64,
    /// Modified timestamps used only for date sorting, aligned with `results` indices.
    sort_modified_cache: Vec<Option<u64>>,
    /// Metadata requests currently queued for date sorting.
    sort_metadata_inflight: HashSet<String>,

    // --- Created-date filter metadata cache ---
    /// Creation timestamps for date-range filtering, aligned with `results` indices.
    created_ts_cache: Vec<CreatedMetadataState>,
    /// Incremented whenever created-date metadata is updated; forces filter cache rebuild.
    created_metadata_epoch: u64,
    /// Metadata requests currently queued for created-date filtering.
    created_metadata_inflight: HashSet<String>,

    /// Service-result count loaded for the active query. This intentionally
    /// excludes client-side tagged result injections used by the tag filter.
    pub service_results_loaded: u32,
    /// Last query/tag-assignment combination used to inject tagged matches.
    pub tagged_results_cache_key: Option<(String, GlobalSearchTagFilter, u64)>,

    // --- Tooltip async worker ---
    pub tooltip_sender: Sender<TooltipRequest>,
    pub tooltip_receiver: Receiver<TooltipResponse>,
    pub tooltip_metadata_inflight: HashSet<String>,
    pub tooltip_thumbnail_inflight: HashSet<String>,
}

impl GlobalSearchState {
    pub fn new(
        sender: Sender<crate::workers::global_search_worker::GlobalSearchRequest>,
        receiver: Receiver<crate::workers::global_search_worker::GlobalSearchResponse>,
    ) -> Self {
        // Placeholder channels — replaced by spawn_tooltip_worker() during bootstrap.
        let (tooltip_sender, _) = std::sync::mpsc::channel::<TooltipRequest>();
        let (_, tooltip_receiver) = std::sync::mpsc::channel::<TooltipResponse>();
        Self {
            sender,
            receiver,
            query: String::new(),
            results: Vec::new(),
            selected_index: None,
            focus_request: false,
            size_cache: LruCache::new(
                NonZeroUsize::new(2000).expect("global_search size_cache size must be non-zero"),
            ),
            tooltip_texture_cache: LruCache::new(
                NonZeroUsize::new(50).expect("tooltip_texture_cache size must be non-zero"),
            ),
            metadata_cache: LruCache::new(
                NonZeroUsize::new(500).expect("metadata_cache size must be non-zero"),
            ),
            category: GlobalSearchCategory::All,
            drive_filter: None,
            sort_mode: GlobalSearchSortMode::Relevance,
            sort_descending: false,
            min_size_mb: None,
            max_size_mb: None,
            created_after: None,
            created_before: None,
            tag_filter: GlobalSearchTagFilter::All,
            created_after_month: 0,
            created_after_day: 0,
            created_after_year: 0,
            created_after_month_text: String::new(),
            created_after_day_text: String::new(),
            created_after_year_text: String::new(),
            created_before_month: 0,
            created_before_day: 0,
            created_before_year: 0,
            created_before_month_text: String::new(),
            created_before_day_text: String::new(),
            created_before_year_text: String::new(),
            active: false,
            opened_at: Instant::now(),
            loading: false,
            pending_query_dispatch_at: None,
            in_flight_query: None,
            in_flight_started_at: None,
            requested_offset: 0,
            requested_limit: 500,
            has_more_results: false,
            available: false,
            last_check: Instant::now(),
            last_status_received_at: Instant::now(),
            last_progress_advance_at: Instant::now(),
            total_indexed: 0,
            session_total_indexed: 0,
            total_matches: None,
            status_volumes: Vec::new(),
            indexing_in_progress: false,
            scroll_offset_y: 0.0,
            last_scroll_offset_y: 0.0,
            last_scroll_time: Instant::now(),
            results_generation: 0,
            cached_filtered_indices: Vec::new(),
            cached_available_drives: Vec::new(),
            filter_cache_key: (
                u64::MAX,
                GlobalSearchFilters {
                    category: GlobalSearchCategory::All,
                    drive_filter: None,
                    min_size_mb: None,
                    max_size_mb: None,
                    created_after: None,
                    created_before: None,
                    tag_filter: GlobalSearchTagFilter::All,
                },
                0,
                0,
            ),
            cached_sorted_indices: Vec::new(),
            sort_cache_key: (
                u64::MAX,
                GlobalSearchFilters {
                    category: GlobalSearchCategory::All,
                    drive_filter: None,
                    min_size_mb: None,
                    max_size_mb: None,
                    created_after: None,
                    created_before: None,
                    tag_filter: GlobalSearchTagFilter::All,
                },
                GlobalSearchSortMode::Relevance,
                false,
                0,
                0,
                0,
            ),
            sort_metadata_epoch: 0,
            sort_modified_cache: Vec::new(),
            sort_metadata_inflight: HashSet::new(),
            created_ts_cache: Vec::new(),
            created_metadata_epoch: 0,
            created_metadata_inflight: HashSet::new(),
            service_results_loaded: 0,
            tagged_results_cache_key: None,
            tooltip_sender,
            tooltip_receiver,
            tooltip_metadata_inflight: HashSet::new(),
            tooltip_thumbnail_inflight: HashSet::new(),
        }
    }

    /// Spawns the tooltip background worker thread and reconnects the channels.
    /// Must be called once during bootstrap after the state is constructed.
    pub fn spawn_tooltip_worker(
        &mut self,
        disk_cache: std::sync::Arc<crate::infrastructure::disk_cache::ThumbnailDiskCache>,
        ctx: &egui::Context,
    ) {
        let (req_tx, req_rx) = std::sync::mpsc::channel::<TooltipRequest>();
        let (resp_tx, resp_rx) = std::sync::mpsc::channel::<TooltipResponse>();
        self.tooltip_sender = req_tx;
        self.tooltip_receiver = resp_rx;

        let ctx = ctx.clone();
        if let Err(e) = std::thread::Builder::new()
            .name("search-tooltip-worker".to_string())
            .spawn(move || {
                while let Ok(req) = req_rx.recv() {
                    match req {
                        TooltipRequest::Metadata(path) => {
                            let meta = std::fs::metadata(&path).ok();
                            let size = meta.as_ref().filter(|m| m.is_file()).map(|m| m.len());
                            let modified_ts = meta
                                .as_ref()
                                .and_then(|m| m.modified().ok())
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            let created_ts = meta
                                .as_ref()
                                .and_then(|m| m.created().ok())
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_secs());
                            if resp_tx
                                .send(TooltipResponse::Metadata {
                                    path,
                                    size,
                                    modified_ts,
                                    created_ts,
                                })
                                .is_err()
                            {
                                break;
                            }
                            ctx.request_repaint();
                        }
                        TooltipRequest::Thumbnail(path) => {
                            let p = std::path::PathBuf::from(&path);
                            let decoded = disk_cache
                                .get_latest(&p)
                                .and_then(|entry| {
                                    image::load_from_memory_with_format(
                                        &entry.data,
                                        image::ImageFormat::WebP,
                                    )
                                    .ok()
                                })
                                .map(|img| {
                                    let rgba = img.to_rgba8();
                                    let w = rgba.width();
                                    let h = rgba.height();
                                    (rgba.into_raw(), w, h)
                                });
                            let response = if let Some((rgba, width, height)) = decoded {
                                TooltipResponse::Thumbnail {
                                    path,
                                    rgba,
                                    width,
                                    height,
                                }
                            } else {
                                TooltipResponse::ThumbnailFailed { path }
                            };

                            if resp_tx.send(response).is_err() {
                                break;
                            }
                            ctx.request_repaint();
                        }
                    }
                }
            })
        {
            log::error!(
                "[GlobalSearch] Failed to spawn search-tooltip-worker thread: {}",
                e
            );
        }
    }

    pub fn clear_transient_results(&mut self) {
        self.results.clear();
        self.cached_filtered_indices.clear();
        self.cached_available_drives.clear();
        self.cached_sorted_indices.clear();
        self.sort_modified_cache.clear();
        self.sort_metadata_inflight.clear();
        self.created_ts_cache.clear();
        self.created_metadata_inflight.clear();
        self.selected_index = None;
        self.has_more_results = false;
        self.total_matches = None;
        self.service_results_loaded = 0;
        self.tagged_results_cache_key = None;
        self.results_generation = self.results_generation.wrapping_add(1);
    }

    pub fn release_transient_results(&mut self) {
        self.clear_transient_results();
        self.results.shrink_to_fit();
        self.cached_filtered_indices.shrink_to_fit();
        self.cached_available_drives.shrink_to_fit();
        self.cached_sorted_indices.shrink_to_fit();
    }

    pub fn clear_transient_caches(&mut self) {
        self.size_cache.clear();
        self.tooltip_texture_cache.clear();
        self.metadata_cache.clear();
        self.sort_modified_cache.clear();
        self.sort_metadata_inflight.clear();
        self.sort_metadata_epoch = self.sort_metadata_epoch.wrapping_add(1);
        self.created_ts_cache.clear();
        self.created_metadata_inflight.clear();
        self.created_metadata_epoch = self.created_metadata_epoch.wrapping_add(1);
        self.cached_sorted_indices.clear();
        self.sort_cache_key.0 = u64::MAX;
    }

    pub fn reset_sort_metadata_for_current_results(&mut self) {
        self.sort_modified_cache.clear();
        self.sort_metadata_inflight.clear();
        self.sort_metadata_epoch = self.sort_metadata_epoch.wrapping_add(1);
        self.created_ts_cache.clear();
        self.created_metadata_inflight.clear();
        self.created_metadata_epoch = self.created_metadata_epoch.wrapping_add(1);
        self.cached_sorted_indices.clear();
        self.sort_cache_key.0 = u64::MAX;
    }

    pub fn invalidate_tag_assignment_dependent_results(&mut self) {
        let service_len = self.service_results_loaded as usize;
        if self.results.len() > service_len {
            self.results.truncate(service_len);
            self.reset_sort_metadata_for_current_results();
        }
        self.tagged_results_cache_key = None;
        self.cached_filtered_indices.clear();
        self.cached_sorted_indices.clear();
        self.results_generation = self.results_generation.wrapping_add(1);
        self.filter_cache_key.0 = u64::MAX;
        self.sort_cache_key.0 = u64::MAX;
    }

    pub fn sync_sort_metadata_len(&mut self) {
        let len = self.results.len();
        if self.sort_modified_cache.len() < len {
            self.sort_modified_cache.resize(len, None);
        } else if self.sort_modified_cache.len() > len {
            self.sort_modified_cache.truncate(len);
        }
    }

    pub fn sort_modified_ts_for_index(&self, idx: usize) -> Option<u64> {
        self.sort_modified_cache.get(idx).copied().flatten()
    }

    pub fn created_ts_for_index(&self, idx: usize) -> Option<u64> {
        match self.created_ts_cache.get(idx).copied() {
            Some(CreatedMetadataState::Available(ts)) => Some(ts),
            _ => None,
        }
    }

    pub fn attach_tooltip_to_sort_metadata_request(&mut self, path: &str) -> bool {
        if !self.sort_metadata_inflight.contains(path) {
            return false;
        }

        self.tooltip_metadata_inflight.insert(path.to_string());
        true
    }

    pub fn apply_sort_metadata(&mut self, path: &str, modified_ts: u64) -> bool {
        let was_sort_request = self.sort_metadata_inflight.remove(path);
        if !was_sort_request && self.sort_mode != GlobalSearchSortMode::ModifiedDate {
            return false;
        }

        self.sync_sort_metadata_len();
        let mut updated = false;
        for idx in 0..self.results.len() {
            if self.results[idx].full_path == path
                && self.sort_modified_cache[idx] != Some(modified_ts)
            {
                self.sort_modified_cache[idx] = Some(modified_ts);
                updated = true;
            }
        }

        if updated {
            self.sort_metadata_epoch = self.sort_metadata_epoch.wrapping_add(1);
        }

        updated
    }

    pub fn sync_created_metadata_len(&mut self) {
        let len = self.results.len();
        if self.created_ts_cache.len() < len {
            self.created_ts_cache
                .resize(len, CreatedMetadataState::Pending);
        } else if self.created_ts_cache.len() > len {
            self.created_ts_cache.truncate(len);
        }
    }

    pub fn apply_created_metadata(&mut self, path: &str, created_ts: Option<u64>) -> bool {
        self.created_metadata_inflight.remove(path);

        self.sync_created_metadata_len();
        let next_state = created_ts
            .map(CreatedMetadataState::Available)
            .unwrap_or(CreatedMetadataState::Unavailable);
        let mut updated = false;
        for idx in 0..self.results.len() {
            if self.results[idx].full_path == path && self.created_ts_cache[idx] != next_state {
                self.created_ts_cache[idx] = next_state;
                updated = true;
            }
        }

        if updated {
            self.created_metadata_epoch = self.created_metadata_epoch.wrapping_add(1);
        }

        updated
    }

    fn queue_missing_created_metadata(&mut self, filtered: &[usize]) {
        self.sync_created_metadata_len();
        let mut remaining =
            MAX_SORT_METADATA_IN_FLIGHT.saturating_sub(self.created_metadata_inflight.len());
        if remaining == 0 {
            return;
        }

        for &idx in filtered {
            if remaining == 0 {
                break;
            }
            if !matches!(
                self.created_ts_cache.get(idx),
                None | Some(CreatedMetadataState::Pending)
            ) {
                continue;
            }

            let Some(path) = self.results.get(idx).map(|result| result.full_path.clone()) else {
                continue;
            };
            if self.created_metadata_inflight.contains(&path) {
                continue;
            }

            self.created_metadata_inflight.insert(path.clone());
            if self.tooltip_metadata_inflight.contains(&path) {
                remaining -= 1;
                continue;
            }

            if self
                .tooltip_sender
                .send(TooltipRequest::Metadata(path.clone()))
                .is_ok()
            {
                remaining -= 1;
            } else {
                self.created_metadata_inflight.remove(&path);
            }
        }
    }

    /// Rebuild the cached filtered indices and available drives only when the
    /// inputs have changed.
    /// Returns a reference to the cached filtered indices.
    pub fn ensure_filter_cache(
        &mut self,
        tag_assignments: &rustc_hash::FxHashMap<String, Vec<i64>>,
        tag_assignments_epoch: u64,
    ) -> &[usize] {
        let filters = GlobalSearchFilters {
            category: self.category,
            drive_filter: self.drive_filter,
            min_size_mb: self.min_size_mb,
            max_size_mb: self.max_size_mb,
            created_after: self.created_after,
            created_before: self.created_before,
            tag_filter: self.tag_filter.clone(),
        };
        let key = (
            self.results_generation,
            filters,
            self.created_metadata_epoch,
            tag_assignments_epoch,
        );
        if self.filter_cache_key != key {
            let min_size_bytes = self.min_size_mb.map(|mb| mb * 1024 * 1024);
            let max_size_bytes = self.max_size_mb.map(|mb| mb * 1024 * 1024);
            self.cached_filtered_indices =
                crate::ui::global_search_overlay::filters::build_filtered_indices(
                    &self.results,
                    self.category,
                    self.drive_filter,
                    min_size_bytes,
                    max_size_bytes,
                    self.created_after,
                    self.created_before,
                    &self.created_ts_cache,
                    &self.tag_filter,
                    tag_assignments,
                );
            self.cached_available_drives =
                crate::ui::global_search_overlay::filters::available_drives(&self.results);

            // Queue metadata for created-date filtering when active.
            if self.created_after.is_some() || self.created_before.is_some() {
                let filtered = self.cached_filtered_indices.clone();
                self.queue_missing_created_metadata(&filtered);
            }

            self.filter_cache_key = key;
        }
        &self.cached_filtered_indices
    }

    /// Returns filtered indices sorted according to the current sort_mode.
    /// Uses a cache key that includes generation, filters, sort_mode, and metadata epochs.
    pub fn ensure_sorted_indices(
        &mut self,
        tag_assignments: &rustc_hash::FxHashMap<String, Vec<i64>>,
        tag_assignments_epoch: u64,
    ) -> &[usize] {
        self.ensure_filter_cache(tag_assignments, tag_assignments_epoch);
        let filters = GlobalSearchFilters {
            category: self.category,
            drive_filter: self.drive_filter,
            min_size_mb: self.min_size_mb,
            max_size_mb: self.max_size_mb,
            created_after: self.created_after,
            created_before: self.created_before,
            tag_filter: self.tag_filter.clone(),
        };
        let key = (
            self.results_generation,
            filters,
            self.sort_mode,
            self.sort_descending,
            self.sort_metadata_epoch,
            self.created_metadata_epoch,
            tag_assignments_epoch,
        );
        if self.sort_cache_key != key {
            let mut sorted = self.cached_filtered_indices.clone();
            if self.sort_mode == GlobalSearchSortMode::ModifiedDate {
                self.queue_missing_sort_metadata(&sorted);
                let cache = &self.sort_modified_cache;
                let descending = self.sort_descending;
                sorted.sort_by(|&a, &b| {
                    let ts_a = cache.get(a).copied().flatten().unwrap_or(0);
                    let ts_b = cache.get(b).copied().flatten().unwrap_or(0);
                    let order = if descending {
                        ts_b.cmp(&ts_a)
                    } else {
                        ts_a.cmp(&ts_b)
                    };
                    order.then_with(|| a.cmp(&b))
                });
            } else if self.sort_mode == GlobalSearchSortMode::Name {
                let results = &self.results;
                let descending = self.sort_descending;
                sorted.sort_by(|&a, &b| {
                    let name_a = results.get(a).map(|r| r.name.as_str()).unwrap_or("");
                    let name_b = results.get(b).map(|r| r.name.as_str()).unwrap_or("");
                    let mut order = name_a.to_lowercase().cmp(&name_b.to_lowercase());
                    if descending {
                        order = order.reverse();
                    }
                    order.then_with(|| a.cmp(&b))
                });
            }
            self.cached_sorted_indices = sorted;
            self.sort_cache_key = key;
        }
        &self.cached_sorted_indices
    }

    fn queue_missing_sort_metadata(&mut self, sorted: &[usize]) {
        self.sync_sort_metadata_len();
        let mut remaining =
            MAX_SORT_METADATA_IN_FLIGHT.saturating_sub(self.sort_metadata_inflight.len());
        if remaining == 0 {
            return;
        }

        for &idx in sorted {
            if remaining == 0 {
                break;
            }
            if self
                .sort_modified_cache
                .get(idx)
                .copied()
                .flatten()
                .is_some()
            {
                continue;
            }

            let Some(path) = self.results.get(idx).map(|result| result.full_path.clone()) else {
                continue;
            };
            if self.sort_metadata_inflight.contains(&path) {
                continue;
            }

            self.sort_metadata_inflight.insert(path.clone());
            if self.tooltip_metadata_inflight.contains(&path) {
                remaining -= 1;
                continue;
            }

            if self
                .tooltip_sender
                .send(TooltipRequest::Metadata(path.clone()))
                .is_ok()
            {
                remaining -= 1;
            } else {
                self.sort_metadata_inflight.remove(&path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{GlobalSearchSortMode, GlobalSearchState, TooltipRequest};
    use mtt_search_protocol::SearchResultItem;

    fn state_with_tooltip_receiver(
    ) -> (GlobalSearchState, std::sync::mpsc::Receiver<TooltipRequest>) {
        let (search_tx, _search_rx) = std::sync::mpsc::channel();
        let (_response_tx, response_rx) = std::sync::mpsc::channel();
        let mut state = GlobalSearchState::new(search_tx, response_rx);
        let (tooltip_tx, tooltip_rx) = std::sync::mpsc::channel();
        state.tooltip_sender = tooltip_tx;
        (state, tooltip_rx)
    }

    #[test]
    fn date_sort_metadata_requests_progress_beyond_tooltip_lru_capacity() {
        let (mut state, tooltip_rx) = state_with_tooltip_receiver();
        state.sort_mode = GlobalSearchSortMode::ModifiedDate;
        state.results = (0..600)
            .map(|idx| SearchResultItem {
                name: format!("file_{idx}.txt"),
                full_path: format!(r"C:\tmp\file_{idx}.txt"),
                is_dir: false,
                size: 0,
            })
            .collect();
        state.results_generation += 1;

        let mut resolved = 0usize;
        loop {
            let assignments = rustc_hash::FxHashMap::default();
            state.ensure_sorted_indices(&assignments, 0);
            let mut batch = Vec::new();
            while let Ok(request) = tooltip_rx.try_recv() {
                match request {
                    TooltipRequest::Metadata(path) => batch.push(path),
                    TooltipRequest::Thumbnail(_) => panic!("unexpected thumbnail request"),
                }
            }

            if batch.is_empty() {
                break;
            }

            assert!(batch.len() <= super::MAX_SORT_METADATA_IN_FLIGHT);
            for path in batch {
                resolved += 1;
                state.apply_sort_metadata(&path, resolved as u64);
            }
        }

        assert_eq!(resolved, 600);
        assert_eq!(
            state
                .sort_modified_cache
                .iter()
                .filter(|modified_ts| modified_ts.is_some())
                .count(),
            600
        );
        assert!(state.sort_metadata_inflight.is_empty());

        let assignments = rustc_hash::FxHashMap::default();
        state.ensure_sorted_indices(&assignments, 0);
        assert!(tooltip_rx.try_recv().is_err());
    }
}
