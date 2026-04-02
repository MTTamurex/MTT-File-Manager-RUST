use eframe::egui;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

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
    pub active: bool,
    pub loading: bool,
    pub pending_query_dispatch_at: Option<Instant>,
    pub in_flight_query: Option<String>,
    pub in_flight_started_at: Option<Instant>,
    pub requested_offset: u32,
    pub requested_limit: u32,
    pub has_more_results: bool,
    pub available: bool,
    pub last_check: Instant,
    pub total_indexed: u64,
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

}

impl GlobalSearchState {
    pub fn new(
        sender: Sender<crate::workers::global_search_worker::GlobalSearchRequest>,
        receiver: Receiver<crate::workers::global_search_worker::GlobalSearchResponse>,
    ) -> Self {
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
            active: false,
            loading: false,
            pending_query_dispatch_at: None,
            in_flight_query: None,
            in_flight_started_at: None,
            requested_offset: 0,
            requested_limit: 200,
            has_more_results: false,
            available: false,
            last_check: Instant::now(),
            total_indexed: 0,
            scroll_offset_y: 0.0,
            last_scroll_offset_y: 0.0,
            last_scroll_time: Instant::now(),
            results_generation: 0,
            cached_filtered_indices: Vec::new(),
            cached_available_drives: Vec::new(),
            filter_cache_key: (u64::MAX, GlobalSearchCategory::All, None),
        }
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
}
