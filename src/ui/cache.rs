//! Cache management for textures and icons
//! Follows .cursorrules: zero allocations in hot path, LRU eviction

use eframe::egui;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

// PERFORMANCE: FxHashSet uses a faster hash function than std::collections::HashSet.
// This is especially beneficial for PathBuf keys which have expensive default hashing.
// FxHash is ~2-4x faster for string-like keys.
// Re-exported for use in other modules.
pub use rustc_hash::FxHashSet;

const DEFAULT_TEXTURE_CACHE_ITEMS: usize = 64;
const DEFAULT_FOLDER_PREVIEW_CACHE_ITEMS: usize = 48;
const DEFAULT_RGBA_CACHE_ITEMS: usize = 32;
const DEFAULT_MAX_CONCURRENT_LOADS: usize = 80;
const DEFAULT_RGBA_BUDGET_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const MIN_DYNAMIC_TEXTURE_CACHE_ITEMS: usize = 48;
pub(crate) const MAX_DYNAMIC_TEXTURE_CACHE_ITEMS: usize = 1500;
pub(crate) const VULKAN_MAX_DYNAMIC_TEXTURE_CACHE_ITEMS: usize = 384;
pub(crate) const MIN_DYNAMIC_FOLDER_PREVIEW_ITEMS: usize = 32;
pub(crate) const MAX_DYNAMIC_FOLDER_PREVIEW_ITEMS: usize = 1500;
pub(crate) const VULKAN_MAX_DYNAMIC_FOLDER_PREVIEW_ITEMS: usize = 256;
pub(crate) const MIN_RGBA_BUDGET_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const DEFAULT_DYNAMIC_RGBA_BUDGET_BYTES: usize = 24 * 1024 * 1024;
pub(crate) const MAX_RGBA_BUDGET_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_THUMBNAIL_LOADING_SET_ITEMS: usize = 1024;
const MAX_PENDING_UPLOAD_SET_ITEMS: usize = 2048;

/// Per-path cooldown for `request_folder_preview_load`. Prevents render-loop
/// thrash when the LRU cap is smaller than the directory's folder set:
/// without this, an evicted preview would be re-requested every frame and
/// each `ctx.load_texture` call accumulates GPU staging that is not freed
/// at the same rate, producing a steady working-set leak.
const FOLDER_PREVIEW_REQUEST_COOLDOWN: Duration = Duration::from_millis(2000);
const FOLDER_PREVIEW_LOADING_STALE_AFTER: Duration = Duration::from_secs(15);
/// Bounded LRU for the cooldown map; ~96B/entry → ~400KB ceiling.
const FOLDER_PREVIEW_DEBOUNCE_CAPACITY: usize = 4096;

/// Per-path cooldown for `request_thumbnail_load_internal`. Mirrors the
/// folder-preview cooldown: an evicted thumbnail texture would otherwise be
/// re-requested every render frame, dispatching new worker jobs whose
/// uploaded textures pop the previously visible ones from the LRU. Each
/// `ctx.load_texture` accumulates GPU staging that the OS releases more
/// slowly than we allocate it, producing a steady working-set leak even
/// when the cache cap is technically large enough to hold every visible
/// path.
// 300 ms is enough to prevent per-frame re-request loops (~60fps × 300ms = 18 retries/s
// maximum), while still allowing stuck items to recover quickly: items dropped from
// the pending queue (because it was full) or evicted from the texture cache can be
// re-requested within 300 ms rather than the previous 2000 ms.
const THUMBNAIL_REQUEST_COOLDOWN: Duration = Duration::from_millis(300);
/// Bounded LRU for the thumbnail cooldown map; ~96B/entry → ~400KB ceiling.
const THUMBNAIL_DEBOUNCE_CAPACITY: usize = 4096;

/// UI-thread counters for thumbnail pipeline diagnostics. Identical purpose
/// to `FolderPreviewTraceCounters` but plain (non-atomic) since every site
/// runs on the UI thread under `&mut CacheManager`.
#[derive(Default)]
pub struct ThumbnailTraceCounters {
    pub req_total: u64,
    pub req_dup_loading: u64,
    pub req_dup_pending: u64,
    pub req_pending_deletion: u64,
    pub ram_cache_hit: u64,
    pub worker_dispatch: u64,
    pub uploads: u64,
    pub upload_already_cached: u64,
    pub upload_evictions: u64,
    pub sample_request_path: Option<PathBuf>,
    pub sample_upload_path: Option<PathBuf>,
    /// path -> count for the request hot-set so we can identify the actual
    /// source paths that are looping.
    pub request_freq: rustc_hash::FxHashMap<PathBuf, u64>,
}

#[derive(Clone, Default)]
pub struct ThumbnailTraceSnapshot {
    pub req_total: u64,
    pub req_dup_loading: u64,
    pub req_dup_pending: u64,
    pub req_pending_deletion: u64,
    pub ram_cache_hit: u64,
    pub worker_dispatch: u64,
    pub uploads: u64,
    pub upload_already_cached: u64,
    pub upload_evictions: u64,
    pub sample_request_path: Option<PathBuf>,
    pub sample_upload_path: Option<PathBuf>,
    pub unique_request_paths: usize,
    pub top_paths: Vec<(PathBuf, u64)>,
}

impl ThumbnailTraceCounters {
    pub fn record_request(&mut self, path: &PathBuf) {
        self.req_total = self.req_total.saturating_add(1);
        if self.sample_request_path.is_none() {
            self.sample_request_path = Some(path.clone());
        }
        // Bound the map: if it grows past 256 unique paths in a single
        // sampling window, just stop counting new ones — the looping ones
        // will already be captured.
        if self.request_freq.len() < 256 || self.request_freq.contains_key(path) {
            *self.request_freq.entry(path.clone()).or_insert(0) += 1;
        }
    }
    pub fn record_dup_loading(&mut self) {
        self.req_dup_loading = self.req_dup_loading.saturating_add(1);
    }
    pub fn record_dup_pending(&mut self) {
        self.req_dup_pending = self.req_dup_pending.saturating_add(1);
    }
    pub fn record_pending_deletion(&mut self) {
        self.req_pending_deletion = self.req_pending_deletion.saturating_add(1);
    }
    pub fn record_ram_cache_hit(&mut self) {
        self.ram_cache_hit = self.ram_cache_hit.saturating_add(1);
    }
    pub fn record_worker_dispatch(&mut self) {
        self.worker_dispatch = self.worker_dispatch.saturating_add(1);
    }
    pub fn record_upload(&mut self, path: &PathBuf) {
        self.uploads = self.uploads.saturating_add(1);
        if self.sample_upload_path.is_none() {
            self.sample_upload_path = Some(path.clone());
        }
    }
    pub fn record_upload_already_cached(&mut self) {
        self.upload_already_cached = self.upload_already_cached.saturating_add(1);
    }
    pub fn record_upload_eviction(&mut self) {
        self.upload_evictions = self.upload_evictions.saturating_add(1);
    }
    pub fn take_snapshot(&mut self) -> ThumbnailTraceSnapshot {
        // Compute top-3 by frequency before clearing.
        let mut entries: Vec<(PathBuf, u64)> = self.request_freq.drain().collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        let unique_request_paths = entries.len();
        entries.truncate(3);

        let snap = ThumbnailTraceSnapshot {
            req_total: self.req_total,
            req_dup_loading: self.req_dup_loading,
            req_dup_pending: self.req_dup_pending,
            req_pending_deletion: self.req_pending_deletion,
            ram_cache_hit: self.ram_cache_hit,
            worker_dispatch: self.worker_dispatch,
            uploads: self.uploads,
            upload_already_cached: self.upload_already_cached,
            upload_evictions: self.upload_evictions,
            sample_request_path: self.sample_request_path.take(),
            sample_upload_path: self.sample_upload_path.take(),
            unique_request_paths,
            top_paths: entries,
        };
        self.req_total = 0;
        self.req_dup_loading = 0;
        self.req_dup_pending = 0;
        self.req_pending_deletion = 0;
        self.ram_cache_hit = 0;
        self.worker_dispatch = 0;
        self.uploads = 0;
        self.upload_already_cached = 0;
        self.upload_evictions = 0;
        snap
    }
}

#[inline]
fn nz_cache_size(size: usize, cache_name: &str) -> NonZeroUsize {
    if size == 0 {
        log::warn!("{} configured with 0 entries; clamping to 1", cache_name);
    }
    NonZeroUsize::new(size.max(1)).expect("size.max(1) is always non-zero")
}

/// Texture cache configuration
pub struct TextureCacheConfig {
    pub max_size: usize,
    pub max_concurrent_loads: usize,
}

impl Default for TextureCacheConfig {
    fn default() -> Self {
        Self {
            max_size: DEFAULT_TEXTURE_CACHE_ITEMS,
            max_concurrent_loads: DEFAULT_MAX_CONCURRENT_LOADS,
        }
    }
}

/// Manages texture caches for thumbnails and icons
pub struct CacheManager {
    pub texture_cache: LruCache<PathBuf, egui::TextureHandle>,
    pub icon_cache: LruCache<String, egui::TextureHandle>,
    pub loading_set: FxHashSet<PathBuf>,
    pub folder_icon_texture: Option<egui::TextureHandle>,
    pub computer_icon: Option<egui::TextureHandle>,
    pub drive_icon_cache: LruCache<String, egui::TextureHandle>,
    /// Cache for folder preview thumbnails (sandwich effect)
    pub folder_preview_cache: LruCache<PathBuf, egui::TextureHandle>,
    /// Set of folder paths currently being loaded
    pub folder_preview_loading: FxHashSet<PathBuf>,
    folder_preview_loading_started: LruCache<PathBuf, Instant>,
    /// Per-path debounce so an evicted folder cannot be re-requested every
    /// frame by the renderer when the LRU cap is smaller than the directory's
    /// folder set. Bounded LRU keeps memory ceiling deterministic.
    folder_preview_request_debounce: LruCache<PathBuf, Instant>,
    /// Per-path debounce for `request_thumbnail_load_internal` — pairs with
    /// `THUMBNAIL_REQUEST_COOLDOWN`. Bounded LRU keeps memory ceiling
    /// deterministic and behaves identically to `folder_preview_request_debounce`.
    thumbnail_request_debounce: LruCache<PathBuf, Instant>,
    /// Set of paths that failed thumbnail extraction (LRU bounded to 1000)
    pub failed_thumbnails: LruCache<PathBuf, ()>,
    /// Set of paths received from worker but waiting for GPU upload
    pub pending_upload_set: FxHashSet<PathBuf>,
    pub(crate) folder_preview_trace:
        Arc<crate::workers::folder_preview_worker::FolderPreviewTraceCounters>,
    pub(crate) thumbnail_trace: ThumbnailTraceCounters,
    /// Tracks the bucket size we ASKED the worker to extract for a given
    /// path, NOT the actual texture dimensions. Used by the slot renderer
    /// to decide whether to re-request at a higher bucket. Comparing against
    /// the cached texture's real dimensions caused infinite re-extraction
    /// loops for images whose native size is smaller than the desired bucket
    /// (e.g. a 256x256 PNG when the UI wants 512: worker keeps returning 256,
    /// slot keeps re-requesting). Bounded to MAX_DYNAMIC_TEXTURE_CACHE_ITEMS.
    pub attempted_thumbnail_bucket: rustc_hash::FxHashMap<PathBuf, u32>,
    /// Tracks paths that have already emitted a "best_effort_accepted" diagnostic
    /// event, so we don't spam the diagnostic log every frame for the same file.
    pub best_effort_notified: rustc_hash::FxHashSet<PathBuf>,
    /// PERFORMANCE: RAM cache for decoded RGBA data (Layer 2 - larger than VRAM cache)
    /// When a texture is evicted from VRAM, the RGBA data remains here for fast re-upload
    /// without needing disk I/O. This is critical for HDD performance during video playback.
    /// Uses `Arc<Vec<u8>>` so cloning from cache to pending queue is O(1) instead of O(pixels).
    pub rgba_data_cache: LruCache<PathBuf, (Arc<Vec<u8>>, u32, u32)>,
    rgba_data_bytes: usize,
    max_rgba_data_bytes: usize,

    config: TextureCacheConfig,
}

impl CacheManager {
    /// Creates a new cache manager with default configuration
    pub fn new() -> Self {
        Self::new_with_folder_preview_trace(Arc::new(
            crate::workers::folder_preview_worker::FolderPreviewTraceCounters::default(),
        ))
    }

    pub fn new_with_folder_preview_trace(
        folder_preview_trace: Arc<
            crate::workers::folder_preview_worker::FolderPreviewTraceCounters,
        >,
    ) -> Self {
        Self {
            // Bounded default keeps enough history for smooth scrolling without runaway RAM.
            texture_cache: LruCache::new(nz_cache_size(
                DEFAULT_TEXTURE_CACHE_ITEMS,
                "texture_cache",
            )),
            icon_cache: LruCache::new(nz_cache_size(100, "icon_cache")),
            loading_set: FxHashSet::default(),
            folder_icon_texture: None,
            computer_icon: None,
            drive_icon_cache: LruCache::new(nz_cache_size(10, "drive_icon_cache")),
            folder_preview_cache: LruCache::new(nz_cache_size(
                DEFAULT_FOLDER_PREVIEW_CACHE_ITEMS,
                "folder_preview_cache",
            )),
            folder_preview_loading: FxHashSet::default(),
            folder_preview_loading_started: LruCache::new(nz_cache_size(
                FOLDER_PREVIEW_DEBOUNCE_CAPACITY,
                "folder_preview_loading_started",
            )),
            folder_preview_request_debounce: LruCache::new(nz_cache_size(
                FOLDER_PREVIEW_DEBOUNCE_CAPACITY,
                "folder_preview_request_debounce",
            )),
            thumbnail_request_debounce: LruCache::new(nz_cache_size(
                THUMBNAIL_DEBOUNCE_CAPACITY,
                "thumbnail_request_debounce",
            )),
            failed_thumbnails: LruCache::new(nz_cache_size(1000, "failed_thumbnails")),
            pending_upload_set: FxHashSet::default(),
            folder_preview_trace,
            thumbnail_trace: ThumbnailTraceCounters::default(),
            attempted_thumbnail_bucket: rustc_hash::FxHashMap::default(),
            best_effort_notified: rustc_hash::FxHashSet::default(),
            rgba_data_cache: LruCache::new(nz_cache_size(
                DEFAULT_RGBA_CACHE_ITEMS,
                "rgba_data_cache",
            )),
            rgba_data_bytes: 0,
            max_rgba_data_bytes: DEFAULT_DYNAMIC_RGBA_BUDGET_BYTES,

            config: TextureCacheConfig::default(),
        }
    }

    /// Creates a cache manager with custom configuration
    pub fn with_config(config: TextureCacheConfig) -> Self {
        Self::with_config_and_folder_preview_trace(
            config,
            Arc::new(crate::workers::folder_preview_worker::FolderPreviewTraceCounters::default()),
        )
    }

    pub fn with_config_and_folder_preview_trace(
        config: TextureCacheConfig,
        folder_preview_trace: Arc<
            crate::workers::folder_preview_worker::FolderPreviewTraceCounters,
        >,
    ) -> Self {
        let rgba_cache_items = (config.max_size * 6 / 5).max(DEFAULT_RGBA_CACHE_ITEMS);
        let rgba_budget_bytes = (config.max_size * 1024 * 1024 / 2)
            .clamp(DEFAULT_RGBA_BUDGET_BYTES, MAX_RGBA_BUDGET_BYTES);

        Self {
            texture_cache: LruCache::new(nz_cache_size(
                config.max_size,
                "texture_cache(config.max_size)",
            )),
            icon_cache: LruCache::new(nz_cache_size(100, "icon_cache")),
            loading_set: FxHashSet::default(),
            folder_icon_texture: None,
            computer_icon: None,
            drive_icon_cache: LruCache::new(nz_cache_size(10, "drive_icon_cache")),
            folder_preview_cache: LruCache::new(nz_cache_size(
                DEFAULT_FOLDER_PREVIEW_CACHE_ITEMS,
                "folder_preview_cache",
            )),
            folder_preview_loading: FxHashSet::default(),
            folder_preview_loading_started: LruCache::new(nz_cache_size(
                FOLDER_PREVIEW_DEBOUNCE_CAPACITY,
                "folder_preview_loading_started",
            )),
            folder_preview_request_debounce: LruCache::new(nz_cache_size(
                FOLDER_PREVIEW_DEBOUNCE_CAPACITY,
                "folder_preview_request_debounce",
            )),
            thumbnail_request_debounce: LruCache::new(nz_cache_size(
                THUMBNAIL_DEBOUNCE_CAPACITY,
                "thumbnail_request_debounce",
            )),
            failed_thumbnails: LruCache::new(nz_cache_size(1000, "failed_thumbnails")),
            pending_upload_set: FxHashSet::default(),
            folder_preview_trace,
            thumbnail_trace: ThumbnailTraceCounters::default(),
            attempted_thumbnail_bucket: rustc_hash::FxHashMap::default(),
            best_effort_notified: rustc_hash::FxHashSet::default(),
            rgba_data_cache: LruCache::new(nz_cache_size(rgba_cache_items, "rgba_data_cache")),
            rgba_data_bytes: 0,
            max_rgba_data_bytes: rgba_budget_bytes,

            config,
        }
    }

    /// Checks if a thumbnail is in the cache
    pub fn has_thumbnail(&self, path: &PathBuf) -> bool {
        self.texture_cache.contains(path)
    }

    /// Gets a thumbnail from the cache
    pub fn get_thumbnail(&mut self, path: &PathBuf) -> Option<&egui::TextureHandle> {
        self.texture_cache.get(path)
    }

    /// Puts a thumbnail in the cache
    pub fn put_thumbnail(&mut self, path: PathBuf, texture: egui::TextureHandle) {
        if self.texture_cache.contains(&path) {
            self.thumbnail_trace.record_upload_already_cached();
            self.texture_cache.put(path, texture);
            return;
        }

        if self.texture_cache.len() >= self.texture_cache.cap().get() {
            if let Some((old_path, _)) = self.texture_cache.pop_lru() {
                // NOTE: Do NOT call forget_attempted_thumbnail_bucket here.
                // Doing so clears the per-path request cooldown debounce,
                // which immediately re-arms the slot to re-request the
                // evicted thumbnail next frame, producing a feedback loop:
                // upload -> evict -> forget cooldown -> slot re-requests ->
                // upload -> evict ... draining GPU staging memory.
                // The attempted_bucket map and cooldown are intentionally
                // sticky; explicit invalidation paths (rename/delete/refresh)
                // call forget_attempted_thumbnail_bucket directly.
                self.pending_upload_set.remove(&old_path);
            }
            self.thumbnail_trace.record_upload_eviction();
        }

        self.texture_cache.put(path, texture);
    }

    /// Puts a thumbnail while preserving visible textures when possible.
    /// `LruCache::put` evicts the current LRU blindly; during thumbnail churn
    /// that can evict a visible tile and immediately trigger another upload on
    /// the next frame. This method first removes an offscreen LRU entry, then
    /// restores any visible entries it had to walk past.
    pub fn put_thumbnail_preserving_visible(
        &mut self,
        path: PathBuf,
        texture: egui::TextureHandle,
        visible_paths: &FxHashSet<PathBuf>,
    ) {
        if self.texture_cache.contains(&path) {
            self.thumbnail_trace.record_upload_already_cached();
            self.texture_cache.put(path, texture);
            return;
        }

        if self.texture_cache.len() >= self.texture_cache.cap().get() {
            let mut protected_entries = Vec::new();
            let mut removed_offscreen = false;

            while let Some((old_path, old_texture)) = self.texture_cache.pop_lru() {
                if visible_paths.contains(&old_path) {
                    protected_entries.push((old_path, old_texture));
                } else {
                    // See note in put_thumbnail: do NOT clear cooldown on
                    // LRU eviction or the slot will re-request next frame.
                    self.pending_upload_set.remove(&old_path);
                    removed_offscreen = true;
                    break;
                }
            }

            for (old_path, old_texture) in protected_entries {
                self.texture_cache.put(old_path, old_texture);
            }

            if removed_offscreen || self.texture_cache.len() >= self.texture_cache.cap().get() {
                self.thumbnail_trace.record_upload_eviction();
            }

            if self.texture_cache.len() >= self.texture_cache.cap().get() {
                if let Some((old_path, _)) = self.texture_cache.pop_lru() {
                    // See note in put_thumbnail: do NOT clear cooldown here.
                    self.pending_upload_set.remove(&old_path);
                }
            }
        }

        self.texture_cache.put(path, texture);
    }

    /// Records the bucket size that was requested from the worker for `path`.
    /// The slot renderer uses this to decide whether a higher-resolution
    /// re-extraction is needed, instead of comparing against the cached
    /// texture's actual dimensions (which can be smaller than the bucket
    /// when the source image is intrinsically small).
    pub fn note_attempted_thumbnail_bucket(&mut self, path: &PathBuf, bucket: u32) {
        // Defensive cap to prevent unbounded growth on degenerate workloads.
        // The map is keyed by paths that exist or have been requested in the
        // current session; the texture cache itself caps at
        // MAX_DYNAMIC_TEXTURE_CACHE_ITEMS, so this is a safe upper bound.
        if self.attempted_thumbnail_bucket.len() >= MAX_DYNAMIC_TEXTURE_CACHE_ITEMS * 2
            && !self.attempted_thumbnail_bucket.contains_key(path)
        {
            self.attempted_thumbnail_bucket.clear();
        }
        let entry = self
            .attempted_thumbnail_bucket
            .entry(path.clone())
            .or_insert(0);
        if bucket > *entry {
            *entry = bucket;
        }
    }

    /// Returns the bucket size most recently requested for `path`, if any.
    pub fn attempted_thumbnail_bucket_for(&self, path: &PathBuf) -> Option<u32> {
        self.attempted_thumbnail_bucket.get(path).copied()
    }

    /// Forgets the attempted-bucket record for `path`. Callers must invoke
    /// this whenever they pop the texture or otherwise force re-extraction
    /// (rename/delete/refresh-button), so the slot renderer will request
    /// fresh extraction with the desired bucket again.
    pub fn forget_attempted_thumbnail_bucket(&mut self, path: &PathBuf) {
        self.attempted_thumbnail_bucket.remove(path);
        self.thumbnail_request_debounce.pop(path);
        self.best_effort_notified.remove(path);
    }

    /// Touches all currently visible thumbnail-related entries so LRU eviction
    /// prefers off-screen assets during large visible grids.
    pub fn promote_visible(&mut self, visible_paths: &FxHashSet<PathBuf>) {
        for path in visible_paths {
            let _ = self.texture_cache.get(path);
            let _ = self.folder_preview_cache.get(path);
            let _ = self.rgba_data_cache.get(path);
        }
    }

    /// Dynamically adjusts thumbnail cache capacity by rebuilding the LRU with a new cap.
    /// Keeps the hottest entries and drops oldest items if the new cap is smaller.
    pub fn retune_texture_cache_capacity(&mut self, requested_items: usize) -> usize {
        let target_items = requested_items
            .clamp(
                MIN_DYNAMIC_TEXTURE_CACHE_ITEMS,
                MAX_DYNAMIC_TEXTURE_CACHE_ITEMS,
            )
            .max(1);

        let current_items = self.texture_cache.cap().get();
        if current_items == target_items {
            return current_items;
        }

        // PERF FIX (4.2): Use LruCache::resize() instead of manually popping all
        // entries and re-inserting into a new cache. resize() is O(evicted) not O(n).
        self.texture_cache
            .resize(nz_cache_size(target_items, "texture_cache(retune)"));

        target_items
    }

    pub fn retune_folder_preview_cache_capacity(&mut self, requested_items: usize) -> usize {
        let target_items = requested_items
            .clamp(
                MIN_DYNAMIC_FOLDER_PREVIEW_ITEMS,
                MAX_DYNAMIC_FOLDER_PREVIEW_ITEMS,
            )
            .max(1);

        let current_items = self.folder_preview_cache.cap().get();
        if current_items == target_items {
            return current_items;
        }

        self.folder_preview_cache
            .resize(nz_cache_size(target_items, "folder_preview_cache(retune)"));

        target_items
    }

    pub fn retune_rgba_budget(&mut self, requested_bytes: usize) -> usize {
        let target_bytes = requested_bytes.clamp(MIN_RGBA_BUDGET_BYTES, MAX_RGBA_BUDGET_BYTES);
        if self.max_rgba_data_bytes != target_bytes {
            self.max_rgba_data_bytes = target_bytes;
            self.enforce_rgba_budget(target_bytes);
        }
        target_bytes
    }

    pub fn retune_rgba_cache_capacity(&mut self, requested_items: usize) -> usize {
        let target_items = requested_items
            .clamp(DEFAULT_RGBA_CACHE_ITEMS, MAX_DYNAMIC_TEXTURE_CACHE_ITEMS)
            .max(1);

        let current_items = self.rgba_data_cache.cap().get();
        if current_items == target_items {
            return current_items;
        }

        while self.rgba_data_cache.len() > target_items {
            if let Some((_, (data, _, _))) = self.rgba_data_cache.pop_lru() {
                self.rgba_data_bytes = self.rgba_data_bytes.saturating_sub(data.len());
            } else {
                self.rgba_data_bytes = 0;
                break;
            }
        }

        self.rgba_data_cache
            .resize(nz_cache_size(target_items, "rgba_data_cache(retune)"));

        target_items
    }

    /// Checks if a thumbnail is being loaded
    pub fn is_loading(&self, path: &PathBuf) -> bool {
        self.loading_set.contains(path)
    }

    /// Starts loading a thumbnail
    pub fn start_loading(&mut self, path: PathBuf) -> bool {
        if self.loading_set.len() < self.config.max_concurrent_loads {
            self.loading_set.insert(path);
            true
        } else {
            false
        }
    }

    /// Finishes loading a thumbnail
    pub fn finish_loading(&mut self, path: &PathBuf) {
        self.loading_set.remove(path);
    }

    /// Checks if a thumbnail is waiting for upload
    pub fn is_pending_upload(&self, path: &PathBuf) -> bool {
        self.pending_upload_set.contains(path)
    }

    /// Marks a thumbnail as waiting for upload
    pub fn start_pending_upload(&mut self, path: PathBuf) {
        if self.pending_upload_set.len() >= MAX_PENDING_UPLOAD_SET_ITEMS
            && !self.pending_upload_set.contains(&path)
        {
            log::warn!(
                "[CACHE] pending_upload_set reached {} entries; clearing stale markers",
                self.pending_upload_set.len()
            );
            self.pending_upload_set.clear();
        }
        self.pending_upload_set.insert(path);
    }

    /// Removes a thumbnail from pending upload status
    pub fn finish_pending_upload(&mut self, path: &PathBuf) {
        self.pending_upload_set.remove(path);
    }

    /// Clears all caches
    pub fn clear_all(&mut self) {
        self.texture_cache.clear();
        self.icon_cache.clear();
        self.loading_set.clear();
        self.drive_icon_cache.clear();
        self.folder_preview_cache.clear();
        self.folder_preview_loading.clear();
        self.folder_preview_loading_started.clear();
        self.failed_thumbnails.clear();
        self.pending_upload_set.clear();
        self.rgba_data_cache.clear();
        self.rgba_data_bytes = 0;
        self.attempted_thumbnail_bucket.clear();
        self.thumbnail_request_debounce.clear();
        self.folder_preview_request_debounce.clear();
        // Note: folder_icon_texture and computer_icon are kept as they're singletons
    }

    /// Releases thumbnail-only state when the current view cannot display
    /// thumbnails at all (This PC, Recycle Bin, etc.). Unlike the normal trim
    /// path, this recreates the LRUs so their internal allocations and old
    /// TextureHandles are dropped immediately.
    pub fn release_thumbnail_caches_for_idle(
        &mut self,
        texture_capacity: usize,
        folder_preview_capacity: usize,
        rgba_capacity: usize,
        rgba_budget_bytes: usize,
    ) -> (usize, usize, usize, usize) {
        let textures_removed = self.texture_cache.len();
        let folder_previews_removed = self.folder_preview_cache.len();
        let rgba_removed = self.rgba_data_cache.len();
        let rgba_bytes_removed = self.rgba_data_bytes;

        self.texture_cache = LruCache::new(nz_cache_size(texture_capacity, "texture_cache(idle)"));
        self.folder_preview_cache = LruCache::new(nz_cache_size(
            folder_preview_capacity,
            "folder_preview_cache(idle)",
        ));
        self.rgba_data_cache = LruCache::new(nz_cache_size(rgba_capacity, "rgba_data_cache(idle)"));
        self.rgba_data_bytes = 0;
        self.max_rgba_data_bytes = rgba_budget_bytes;

        self.loading_set.clear();
        self.loading_set.shrink_to_fit();
        self.folder_preview_loading.clear();
        self.folder_preview_loading.shrink_to_fit();
        self.folder_preview_loading_started.clear();
        self.pending_upload_set.clear();
        self.pending_upload_set.shrink_to_fit();
        self.attempted_thumbnail_bucket.clear();
        self.attempted_thumbnail_bucket.shrink_to_fit();
        self.best_effort_notified.clear();
        self.best_effort_notified.shrink_to_fit();
        self.thumbnail_request_debounce = LruCache::new(nz_cache_size(
            THUMBNAIL_DEBOUNCE_CAPACITY,
            "thumbnail_request_debounce(idle)",
        ));
        self.folder_preview_request_debounce = LruCache::new(nz_cache_size(
            FOLDER_PREVIEW_DEBOUNCE_CAPACITY,
            "folder_preview_request_debounce(idle)",
        ));

        (
            textures_removed,
            rgba_removed,
            folder_previews_removed,
            rgba_bytes_removed,
        )
    }

    // ========== RAM Cache Methods (Layer 2 - RGBA Data) ==========

    /// Checks if RGBA data is in the RAM cache
    pub fn has_rgba_data(&self, path: &PathBuf) -> bool {
        self.rgba_data_cache.contains(path)
    }

    /// Gets RGBA data from the RAM cache
    pub fn get_rgba_data(&mut self, path: &PathBuf) -> Option<&(Arc<Vec<u8>>, u32, u32)> {
        self.rgba_data_cache.get(path)
    }

    /// Stores RGBA data in the RAM cache
    pub fn put_rgba_data(&mut self, path: PathBuf, data: Arc<Vec<u8>>, width: u32, height: u32) {
        let new_bytes = data.len();

        if let Some((old_data, _, _)) = self.rgba_data_cache.pop(&path) {
            self.rgba_data_bytes = self.rgba_data_bytes.saturating_sub(old_data.len());
        }

        if let Some((_, (evicted_data, _, _))) =
            self.rgba_data_cache.push(path, (data, width, height))
        {
            self.rgba_data_bytes = self.rgba_data_bytes.saturating_sub(evicted_data.len());
        }
        self.rgba_data_bytes = self.rgba_data_bytes.saturating_add(new_bytes);
        self.enforce_rgba_budget(self.max_rgba_data_bytes);
    }

    /// Removes RGBA data for a specific path and updates memory accounting.
    pub fn pop_rgba_data(&mut self, path: &PathBuf) -> Option<(Arc<Vec<u8>>, u32, u32)> {
        if let Some((data, width, height)) = self.rgba_data_cache.pop(path) {
            self.rgba_data_bytes = self.rgba_data_bytes.saturating_sub(data.len());
            Some((data, width, height))
        } else {
            None
        }
    }

    /// Trims thumbnail-related caches to target sizes.
    /// Returns `(textures_removed, rgba_removed, folder_previews_removed)`.
    pub fn trim_thumbnail_caches(
        &mut self,
        target_texture_items: usize,
        target_rgba_bytes: usize,
        target_folder_preview_items: usize,
        visible_paths: Option<&FxHashSet<PathBuf>>,
    ) -> (usize, usize, usize) {
        let mut textures_removed = 0;
        let mut rgba_removed = 0;
        let mut folder_previews_removed = 0;

        if let Some(visible_paths) = visible_paths {
            self.promote_visible(visible_paths);
        }

        while self.texture_cache.len() > target_texture_items {
            if let Some((path, _)) = self.texture_cache.pop_lru() {
                self.pending_upload_set.remove(&path);
                textures_removed += 1;
            } else {
                break;
            }
        }

        while self.folder_preview_cache.len() > target_folder_preview_items {
            if self.folder_preview_cache.pop_lru().is_some() {
                folder_previews_removed += 1;
            } else {
                break;
            }
        }

        while self.rgba_data_bytes > target_rgba_bytes {
            if let Some((_, (data, _, _))) = self.rgba_data_cache.pop_lru() {
                self.rgba_data_bytes = self.rgba_data_bytes.saturating_sub(data.len());
                rgba_removed += 1;
            } else {
                self.rgba_data_bytes = 0;
                break;
            }
        }

        (textures_removed, rgba_removed, folder_previews_removed)
    }

    fn enforce_rgba_budget(&mut self, budget_bytes: usize) {
        while self.rgba_data_bytes > budget_bytes {
            if let Some((_, (data, _, _))) = self.rgba_data_cache.pop_lru() {
                self.rgba_data_bytes = self.rgba_data_bytes.saturating_sub(data.len());
            } else {
                self.rgba_data_bytes = 0;
                break;
            }
        }
    }

    /// Marks a path as having failed thumbnail extraction
    pub fn mark_as_failed(&mut self, path: PathBuf) {
        self.failed_thumbnails.put(path, ());
    }

    /// Checks if a path has previously failed thumbnail extraction
    pub fn is_failed(&self, path: &PathBuf) -> bool {
        self.failed_thumbnails.contains(path)
    }

    /// Clears the failure status for all paths
    pub fn clear_failed(&mut self) {
        self.failed_thumbnails.clear();
    }

    // ========== Folder Preview Methods (Native Windows Shell) ==========

    /// Gets folder preview from cache
    pub fn get_folder_preview(&mut self, path: &PathBuf) -> Option<&egui::TextureHandle> {
        self.folder_preview_cache.get(path)
    }

    /// Checks if folder preview is in cache
    pub fn has_folder_preview(&self, path: &PathBuf) -> bool {
        self.folder_preview_cache.contains(path)
    }

    /// Stores folder preview in cache
    pub fn put_folder_preview(&mut self, path: PathBuf, texture: egui::TextureHandle) {
        if self.folder_preview_cache.len() >= self.folder_preview_cache.cap().get()
            && !self.folder_preview_cache.contains(&path)
        {
            self.folder_preview_trace.record_lru_eviction();
        }
        self.folder_preview_cache.put(path, texture);
    }

    /// Checks if folder preview is currently being loaded
    pub fn is_folder_preview_loading(&self, path: &PathBuf) -> bool {
        self.folder_preview_loading.contains(path)
    }

    /// Returns true if a request for `path` should be skipped because another
    /// request was successfully enqueued within `FOLDER_PREVIEW_REQUEST_COOLDOWN`.
    /// IMPORTANT: this is a pure read — it does NOT poison the cooldown when
    /// the caller cannot enqueue (bounded worker channel full, loading set
    /// rejection, etc.). Callers MUST invoke [`note_folder_preview_request_sent`]
    /// after a request actually reaches the worker.
    pub fn should_throttle_folder_preview_request(&mut self, path: &PathBuf) -> bool {
        let now = Instant::now();
        if let Some(last) = self.folder_preview_request_debounce.get(path) {
            return now.duration_since(*last) < FOLDER_PREVIEW_REQUEST_COOLDOWN;
        }
        false
    }

    /// Records the timestamp of a successfully-enqueued folder preview request.
    /// Pair with [`should_throttle_folder_preview_request`] — only call once the
    /// path is committed to the worker pipeline so transient failures (channel
    /// full, loading-set rejection) don't lock the path out for 2s.
    pub fn note_folder_preview_request_sent(&mut self, path: &PathBuf) {
        self.folder_preview_request_debounce
            .put(path.clone(), Instant::now());
    }

    /// Clears the folder-preview cooldown entry for `path`. Explicit
    /// invalidation and full-cache refresh paths need the next request to go
    /// through immediately instead of waiting for the debounce window.
    pub fn forget_folder_preview_request_cooldown(&mut self, path: &PathBuf) {
        self.folder_preview_request_debounce.pop(path);
    }

    /// Returns true if a `request_thumbnail_load_internal` call for `path`
    /// should be skipped because another request was successfully committed
    /// (worker dispatch or RAM-cache pending push) within
    /// `THUMBNAIL_REQUEST_COOLDOWN`. Pure read — does NOT poison the cooldown
    /// when the caller cannot enqueue.
    pub fn should_throttle_thumbnail_request(&mut self, path: &PathBuf) -> bool {
        let now = Instant::now();
        if let Some(last) = self.thumbnail_request_debounce.get(path) {
            return now.duration_since(*last) < THUMBNAIL_REQUEST_COOLDOWN;
        }
        false
    }

    /// Records the timestamp of a successfully-committed thumbnail request
    /// (RAM-cache pending push or worker dispatch). Only call after the path
    /// is in flight so transient failures don't lock the path out for the
    /// thumbnail request cooldown.
    pub fn note_thumbnail_request_sent(&mut self, path: &PathBuf) {
        self.thumbnail_request_debounce
            .put(path.clone(), Instant::now());
    }

    /// Clears the cooldown entry for `path`. Callers MUST invoke this when the
    /// path is invalidated (rename/delete/refresh-button) so the next request
    /// is not silently throttled.
    pub fn forget_thumbnail_request_cooldown(&mut self, path: &PathBuf) {
        self.thumbnail_request_debounce.pop(path);
    }

    /// Starts loading a folder preview (returns false if too many loads in progress)
    pub fn start_folder_preview_loading(&mut self, path: PathBuf) -> bool {
        if self.folder_preview_loading.contains(&path) {
            return false;
        }

        // Allow deeper queueing so initial folders can request previews without waiting.
        if self.folder_preview_loading.len() < 120 {
            let inserted = self.folder_preview_loading.insert(path.clone());
            if inserted {
                self.folder_preview_loading_started
                    .put(path, Instant::now());
            }
            inserted
        } else {
            false
        }
    }

    /// Finishes loading a folder preview
    pub fn finish_folder_preview_loading(&mut self, path: &PathBuf) {
        self.folder_preview_loading.remove(path);
        self.folder_preview_loading_started.pop(path);
    }

    pub fn prune_stale_folder_preview_loading(&mut self) -> usize {
        let now = Instant::now();
        let stale_paths: Vec<PathBuf> = self
            .folder_preview_loading
            .iter()
            .filter(|path| {
                self.folder_preview_loading_started
                    .peek(path.as_path())
                    .is_none_or(|started| {
                        now.duration_since(*started) >= FOLDER_PREVIEW_LOADING_STALE_AFTER
                    })
            })
            .cloned()
            .collect();

        for path in &stale_paths {
            self.folder_preview_loading.remove(path);
            self.folder_preview_loading_started.pop(path);
            self.forget_folder_preview_request_cooldown(path);
        }

        stale_paths.len()
    }

    /// Invalidates a folder preview (removes from cache and loading set)
    /// Called when folder contents change to trigger reload
    pub fn invalidate_folder_preview(&mut self, path: &PathBuf) {
        self.folder_preview_trace.record_invalidation();
        self.folder_preview_cache.pop(path);
        self.folder_preview_loading.remove(path);
        self.folder_preview_loading_started.pop(path);
        self.forget_folder_preview_request_cooldown(path);
    }
    /// Estimates VRAM usage in bytes
    pub fn estimate_vram_usage(&self) -> usize {
        let texture_usage: usize = self
            .texture_cache
            .iter()
            .map(|(_, tex)| {
                let size = tex.size();
                size[0] * size[1] * 4 // RGBA = 4 bytes per pixel
            })
            .sum();

        let icon_usage: usize = self
            .icon_cache
            .iter()
            .map(|(_, tex)| {
                let size = tex.size();
                size[0] * size[1] * 4
            })
            .sum();

        let drive_icon_usage: usize = self
            .drive_icon_cache
            .iter()
            .map(|(_, tex)| {
                let size = tex.size();
                size[0] * size[1] * 4
            })
            .sum();

        let folder_preview_usage: usize = self
            .folder_preview_cache
            .iter()
            .map(|(_, tex)| {
                let size = tex.size();
                size[0] * size[1] * 4
            })
            .sum();

        texture_usage + folder_preview_usage + icon_usage + drive_icon_usage
    }

    /// Estimates RAM usage by the RGBA data cache in bytes
    pub fn estimate_ram_cache_usage(&self) -> usize {
        self.rgba_data_bytes
    }

    /// Gets or creates a drive icon
    pub fn get_drive_icon(
        &mut self,
        ctx: &egui::Context,
        disk_path: &str,
        extract_fn: impl Fn(&str) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>>,
    ) -> Option<egui::TextureHandle> {
        if let Some(texture) = self.drive_icon_cache.get(disk_path) {
            return Some(texture.clone());
        }

        match extract_fn(disk_path) {
            Ok((rgba_data, width, height)) => {
                let texture = ctx.load_texture(
                    format!("drive_{}", disk_path),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba_data,
                    ),
                    egui::TextureOptions::LINEAR,
                );

                let cloned = texture.clone();
                self.drive_icon_cache.put(disk_path.to_string(), texture);
                Some(cloned)
            }
            Err(_) => None,
        }
    }

    /// Gets or creates a file icon
    pub fn get_file_icon(
        &mut self,
        ctx: &egui::Context,
        path: &PathBuf,
        extract_fn: impl Fn(&PathBuf) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>>,
        extension: &str,
    ) -> Option<egui::TextureHandle> {
        // Decide cache key: full path for executables, extension for others
        let cache_key = if matches!(extension, "exe" | "lnk" | "ico") {
            // Cache by full path - each executable has a unique icon
            path.to_string_lossy().to_string()
        } else {
            // Cache by extension - all .txt share the same icon
            format!(".{}", extension)
        };

        // Cache hit? Clone handle (cheap)
        if let Some(texture) = self.icon_cache.get(&cache_key) {
            return Some(texture.clone());
        }

        // Cache miss -> load icon
        match extract_fn(path) {
            Ok((rgba_data, width, height)) => {
                let texture = ctx.load_texture(
                    format!("icon_{}", cache_key),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba_data,
                    ),
                    egui::TextureOptions::LINEAR,
                );

                let cloned = texture.clone();
                self.icon_cache.put(cache_key, texture);
                Some(cloned)
            }
            Err(_) => None,
        }
    }

    /// Pre-sets the folder icon from custom composed RGBA data.
    pub fn set_folder_icon(&mut self, ctx: &egui::Context, pixels: &[u8], width: u32, height: u32) {
        let texture = ctx.load_texture(
            "folder_icon_composed",
            egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], pixels),
            egui::TextureOptions::LINEAR,
        );
        self.folder_icon_texture = Some(texture);
    }

    /// Ensures computer icon is loaded
    pub fn ensure_computer_icon(
        &mut self,
        ctx: &egui::Context,
        extract_fn: impl Fn() -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>>,
    ) {
        if self.computer_icon.is_some() {
            return;
        }

        match extract_fn() {
            Ok((data, width, height)) => {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    &data,
                );

                self.computer_icon =
                    Some(ctx.load_texture("computer_icon", image, egui::TextureOptions::LINEAR));
            }
            Err(_) => {
                // Fallback
            }
        }
    }
}

impl Default for CacheManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_manager_creation() {
        let cache = CacheManager::new();
        assert_eq!(cache.texture_cache.len(), 0);
        assert_eq!(cache.icon_cache.len(), 0);
        assert!(cache.loading_set.is_empty());
    }

    #[test]
    fn test_loading_management() {
        let mut cache = CacheManager::new();
        let path = PathBuf::from("test.txt");

        assert!(!cache.is_loading(&path));
        assert!(cache.start_loading(path.clone()));
        assert!(cache.is_loading(&path));

        cache.finish_loading(&path);
        assert!(!cache.is_loading(&path));
    }

    #[test]
    fn test_vram_estimation() {
        let cache = CacheManager::new();
        let usage = cache.estimate_vram_usage();
        assert_eq!(usage, 0); // Empty cache
    }

    #[test]
    fn test_cache_clear() {
        let mut cache = CacheManager::new();
        cache.clear_all();
        assert_eq!(cache.texture_cache.len(), 0);
        assert_eq!(cache.icon_cache.len(), 0);
        assert!(cache.loading_set.is_empty());
    }

    #[test]
    fn test_rgba_accounting_updates_on_insert_and_remove() {
        let mut cache = CacheManager::new();
        let path = PathBuf::from("img.webp");

        cache.put_rgba_data(path.clone(), Arc::new(vec![1; 16]), 2, 2);
        assert_eq!(cache.estimate_ram_cache_usage(), 16);

        cache.put_rgba_data(path.clone(), Arc::new(vec![2; 8]), 2, 1);
        assert_eq!(cache.estimate_ram_cache_usage(), 8);

        let _ = cache.pop_rgba_data(&path);
        assert_eq!(cache.estimate_ram_cache_usage(), 0);
    }

    #[test]
    fn test_rgba_accounting_updates_on_lru_capacity_eviction() {
        let mut cache = CacheManager::new();

        for idx in 0..=DEFAULT_RGBA_CACHE_ITEMS {
            cache.put_rgba_data(
                PathBuf::from(format!("img_{idx}.webp")),
                Arc::new(vec![1; 4]),
                1,
                1,
            );
        }

        assert_eq!(cache.rgba_data_cache.len(), DEFAULT_RGBA_CACHE_ITEMS);
        assert_eq!(
            cache.estimate_ram_cache_usage(),
            DEFAULT_RGBA_CACHE_ITEMS * 4
        );
    }

    #[test]
    fn test_rgba_accounting_updates_when_capacity_shrinks() {
        let mut cache = CacheManager::new();
        cache.retune_rgba_cache_capacity(100);

        for idx in 0..100 {
            cache.put_rgba_data(
                PathBuf::from(format!("img_{idx}.webp")),
                Arc::new(vec![1; 4]),
                1,
                1,
            );
        }

        assert_eq!(cache.estimate_ram_cache_usage(), 400);

        cache.retune_rgba_cache_capacity(DEFAULT_RGBA_CACHE_ITEMS);

        assert_eq!(cache.rgba_data_cache.len(), DEFAULT_RGBA_CACHE_ITEMS);
        assert_eq!(
            cache.estimate_ram_cache_usage(),
            DEFAULT_RGBA_CACHE_ITEMS * 4
        );
    }
}
