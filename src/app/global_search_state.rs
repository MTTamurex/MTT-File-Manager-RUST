use eframe::egui;
use lru::LruCache;
use rayon::prelude::*;
use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

const MAX_SORT_METADATA_PER_FRAME: usize = 32;

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
    filter_cache_key: (u64, GlobalSearchCategory, Option<char>),

    // --- Cached sorted indices (includes filter + sort) ---
    pub cached_sorted_indices: Vec<usize>,
    sort_cache_key: (
        u64,
        GlobalSearchCategory,
        Option<char>,
        GlobalSearchSortMode,
        bool,
        u64,
    ),
    /// Incremented whenever new metadata is loaded for sorting; forces a re-sort next frame.
    sort_metadata_epoch: u64,

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
            filter_cache_key: (u64::MAX, GlobalSearchCategory::All, None),
            cached_sorted_indices: Vec::new(),
            sort_cache_key: (
                u64::MAX,
                GlobalSearchCategory::All,
                None,
                GlobalSearchSortMode::Relevance,
                false,
                0,
            ),
            sort_metadata_epoch: 0,
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
                            if resp_tx
                                .send(TooltipResponse::Metadata {
                                    path,
                                    size,
                                    modified_ts,
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
        self.selected_index = None;
        self.has_more_results = false;
        self.total_matches = None;
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
        self.sort_metadata_epoch = 0;
    }

    /// Rebuild the cached filtered indices and available drives only when the
    /// inputs have changed (results generation, category, or drive filter).
    /// Returns a reference to the cached filtered indices.
    pub fn ensure_filter_cache(&mut self) -> &[usize] {
        let key = (self.results_generation, self.category, self.drive_filter);
        if self.filter_cache_key != key {
            self.cached_filtered_indices =
                crate::ui::global_search_overlay::filters::build_filtered_indices(
                    &self.results,
                    self.category,
                    self.drive_filter,
                );
            self.cached_available_drives =
                crate::ui::global_search_overlay::filters::available_drives(&self.results);
            self.filter_cache_key = key;
        }
        &self.cached_filtered_indices
    }

    /// Returns filtered indices sorted according to the current sort_mode.
    /// Uses a cache key that includes generation, category, drive_filter, sort_mode, and sort_descending.
    pub fn ensure_sorted_indices(&mut self) -> &[usize] {
        self.ensure_filter_cache();
        let key = (
            self.results_generation,
            self.category,
            self.drive_filter,
            self.sort_mode,
            self.sort_descending,
            self.sort_metadata_epoch,
        );
        if self.sort_cache_key != key {
            let mut sorted = self.cached_filtered_indices.clone();
            if self.sort_mode == GlobalSearchSortMode::ModifiedDate {
                // Budgeted metadata loading: avoid UI stalls by reading only a few files per frame.
                // Missing entries are fetched in parallel via rayon and cached; the epoch bump
                // triggers a re-sort on the next frame until every item is resolved.
                let missing: Vec<String> = sorted
                    .iter()
                    .filter(|&&idx| {
                        self.metadata_cache
                            .get(&self.results[idx].full_path)
                            .is_none()
                    })
                    .take(MAX_SORT_METADATA_PER_FRAME)
                    .map(|&idx| self.results[idx].full_path.clone())
                    .collect();

                if !missing.is_empty() {
                    let fetched: Vec<(String, u64)> = missing
                        .into_par_iter()
                        .map(|path| {
                            let ts = std::fs::metadata(&path)
                                .ok()
                                .and_then(|m| m.modified().ok())
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            (path, ts)
                        })
                        .collect();

                    for (path, ts) in fetched {
                        self.metadata_cache.put(path, ts);
                    }
                    self.sort_metadata_epoch = self.sort_metadata_epoch.wrapping_add(1);
                }

                sorted.sort_by(|&a, &b| {
                    let ts_a = self
                        .metadata_cache
                        .get(&self.results[a].full_path)
                        .copied()
                        .unwrap_or(0);
                    let ts_b = self
                        .metadata_cache
                        .get(&self.results[b].full_path)
                        .copied()
                        .unwrap_or(0);
                    ts_a.cmp(&ts_b)
                });
                if self.sort_descending {
                    sorted.reverse();
                }
            }
            self.cached_sorted_indices = sorted;
            self.sort_cache_key = key;
        }
        &self.cached_sorted_indices
    }
}
