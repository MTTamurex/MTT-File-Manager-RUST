use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::domain::file_entry::FileEntry;
use crate::domain::file_entry::ViewMode;
use crate::domain::thumbnail::ThumbnailData;
use crate::ui::cache::{
    FxHashSet, DEFAULT_DYNAMIC_RGBA_BUDGET_BYTES, LOW_RAM_GPU_MAX_DYNAMIC_FOLDER_PREVIEW_ITEMS,
    LOW_RAM_GPU_MAX_DYNAMIC_TEXTURE_CACHE_ITEMS, MAX_DYNAMIC_FOLDER_PREVIEW_ITEMS,
    MAX_DYNAMIC_TEXTURE_CACHE_ITEMS, MAX_RGBA_BUDGET_BYTES, MIN_DYNAMIC_FOLDER_PREVIEW_ITEMS,
    MIN_DYNAMIC_TEXTURE_CACHE_ITEMS, MIN_RGBA_BUDGET_BYTES,
};
use crate::workers::thumbnail::processing::get_bucket_size;

use super::ImageViewerApp;

const BASE_PENDING_THUMBNAILS: usize = 64;
const MIN_DYNAMIC_PENDING_THUMBNAILS: usize = 16;
const MAX_DYNAMIC_PENDING_THUMBNAILS: usize = 1024;
const MAX_PENDING_THUMBNAIL_RGBA_BYTES: usize = 64 * 1024 * 1024;
const LOW_RAM_GPU_MAX_PENDING_THUMBNAIL_RGBA_BYTES: usize = 16 * 1024 * 1024;
const LOW_RAM_GPU_RGBA_BUDGET_FLOOR_BYTES: usize = MIN_RGBA_BUDGET_BYTES;
const LOW_RAM_GPU_MAX_RGBA_BUDGET_BYTES: usize = 8 * 1024 * 1024;
const MEMORY_TRACE_INTERVAL: Duration = Duration::from_secs(5);
const IDLE_THUMBNAIL_TEXTURE_KEEP: usize = 8;
const IDLE_FOLDER_PREVIEW_KEEP: usize = 0;
const IDLE_RGBA_BUDGET_BYTES: usize = 4 * 1024 * 1024;
const IDLE_PENDING_THUMBNAILS: usize = 1;
const NAVIGATION_RGBA_CACHE_ITEMS: usize = 32;
const INACTIVE_THUMBNAIL_CACHE_ITEMS: usize = 1;
const WORKING_SET_TRIM_FOLLOW_UP_DELAYS: &[Duration] = &[
    Duration::from_millis(750),
    Duration::from_millis(2500),
    Duration::from_millis(6000),
];
const WORKING_SET_TRIM_MIN_INTERVAL: Duration = Duration::from_secs(10);
const WORKING_SET_TRIM_ACTIVITY_GRACE: Duration = Duration::from_secs(1);
const LOW_RAM_GPU_IDLE_WS_TRIM_AFTER: Duration = Duration::from_secs(8);
const LOW_RAM_GPU_IDLE_WS_TRIM_MIN_BYTES: u64 = 24 * 1024 * 1024;
const WORKING_SET_TRIM_EFFECTIVE_REDUCTION_BYTES: u64 = 1024 * 1024;
const BACKGROUND_WS_TRIM_REARM_GROWTH_BYTES: u64 = 8 * 1024 * 1024;
const SOFT_MEMORY_LIMIT_BYTES: u64 = 550 * 1024 * 1024;
const HARD_MEMORY_LIMIT_BYTES: u64 = 700 * 1024 * 1024;
static WORKING_SET_TRIM_BLOCKED: AtomicBool = AtomicBool::new(false);
static WORKING_SET_TRIM_EPOCH: AtomicU64 = AtomicU64::new(0);
static WORKING_SET_TRIM_SUCCESS_COUNT: AtomicU64 = AtomicU64::new(0);
static LAST_EFFECTIVE_WORKING_SET_TRIM_BYTES: AtomicU64 = AtomicU64::new(0);
static LAST_WORKING_SET_TRIM_REQUEST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
static WORKING_SET_TRIM_EXECUTION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone, Copy, Debug)]
struct ProcessMemorySnapshot {
    working_set_bytes: u64,
    private_usage_bytes: u64,
}

fn bytes_to_mb(bytes: u64) -> f64 {
    bytes as f64 / 1024.0 / 1024.0
}

fn memory_trace_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("MTT_MEMORY_TRACE")
            .map(|value| {
                let value = value.trim();
                value == "1"
                    || value.eq_ignore_ascii_case("true")
                    || value.eq_ignore_ascii_case("yes")
                    || value.eq_ignore_ascii_case("on")
            })
            .unwrap_or(false)
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MemoryPressure {
    None,
    Soft,
    Hard,
}

fn classify_memory_pressure(snapshot: ProcessMemorySnapshot) -> MemoryPressure {
    let pressure_bytes = snapshot.working_set_bytes.max(snapshot.private_usage_bytes);
    if pressure_bytes >= HARD_MEMORY_LIMIT_BYTES {
        MemoryPressure::Hard
    } else if pressure_bytes >= SOFT_MEMORY_LIMIT_BYTES {
        MemoryPressure::Soft
    } else {
        MemoryPressure::None
    }
}

fn working_set_trim_cancelled(blocked: bool, current_epoch: u64, scheduled_epoch: u64) -> bool {
    blocked || current_epoch != scheduled_epoch
}

fn background_trim_should_rearm(
    active: bool,
    pending: bool,
    working_set_bytes: u64,
    stable_working_set_bytes: u64,
) -> bool {
    active
        && !pending
        && working_set_bytes >= LOW_RAM_GPU_IDLE_WS_TRIM_MIN_BYTES
        && working_set_bytes.saturating_sub(stable_working_set_bytes)
            >= BACKGROUND_WS_TRIM_REARM_GROWTH_BYTES
}

fn working_set_trim_was_effective(
    before: ProcessMemorySnapshot,
    after: ProcessMemorySnapshot,
) -> bool {
    before
        .working_set_bytes
        .saturating_sub(after.working_set_bytes)
        >= WORKING_SET_TRIM_EFFECTIVE_REDUCTION_BYTES
}

fn working_set_trim_execution_lock() -> &'static Mutex<()> {
    WORKING_SET_TRIM_EXECUTION_LOCK.get_or_init(|| Mutex::new(()))
}

pub(crate) fn cancel_pending_working_set_trim_for_native_restore() {
    let _execution_guard = working_set_trim_execution_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    WORKING_SET_TRIM_EPOCH.fetch_add(1, Ordering::AcqRel);
}

fn backend_uses_low_ram_gpu_policy(active_gpu_backend: &str) -> bool {
    matches!(active_gpu_backend, "glow" | "Vulkan" | "Dx12")
}

fn backend_uses_conservative_thumbnail_upload_policy(active_gpu_backend: &str) -> bool {
    matches!(active_gpu_backend, "Vulkan" | "Dx12")
}

fn pending_thumbnail_eviction_index(
    pending: &std::collections::VecDeque<ThumbnailData>,
    visible_paths: Option<&FxHashSet<std::path::PathBuf>>,
    selected_path: Option<&std::path::PathBuf>,
) -> Option<usize> {
    pending
        .iter()
        .position(|thumbnail| {
            selected_path != Some(&thumbnail.path)
                && !visible_paths.is_some_and(|visible| visible.contains(&thumbnail.path))
        })
        .or_else(|| {
            if visible_paths.is_none() {
                pending
                    .iter()
                    .rposition(|thumbnail| selected_path != Some(&thumbnail.path))
            } else {
                None
            }
        })
}

fn trim_pending_thumbnail_queue(
    pending: &mut std::collections::VecDeque<ThumbnailData>,
    initial_pending_bytes: usize,
    max_items: usize,
    max_bytes: usize,
    visible_paths: Option<&FxHashSet<std::path::PathBuf>>,
    selected_path: Option<&std::path::PathBuf>,
) -> (Vec<ThumbnailData>, usize) {
    let mut pending_bytes = initial_pending_bytes;
    let mut removed = Vec::new();

    while pending.len() > max_items || pending_bytes > max_bytes {
        let Some(index) = pending_thumbnail_eviction_index(pending, visible_paths, selected_path)
        else {
            break;
        };
        let Some(thumbnail) = pending.remove(index) else {
            break;
        };
        pending_bytes = pending_bytes.saturating_sub(thumbnail.image_data.len());
        removed.push(thumbnail);
    }

    (removed, pending_bytes)
}

fn panel_thumbnail_caches_active(
    view_mode: ViewMode,
    is_computer_view: bool,
    is_recycle_bin_view: bool,
    item_count: usize,
) -> bool {
    matches!(
        view_mode,
        ViewMode::Grid | ViewMode::List | ViewMode::ColumnList | ViewMode::Miller
    ) && !is_computer_view
        && !is_recycle_bin_view
        && item_count > 0
}

fn detail_panel_thumbnail_active(
    show_preview_panel: bool,
    selection_count: usize,
    selected_file: Option<&FileEntry>,
) -> bool {
    show_preview_panel && selection_count <= 1 && selected_file.is_some_and(FileEntry::is_media)
}

fn visible_count_from_range(
    item_count: usize,
    visible_index_range: Option<(usize, usize)>,
) -> Option<usize> {
    let (min_idx, max_idx) = visible_index_range?;
    if item_count == 0 {
        return None;
    }

    let max_idx = max_idx.min(item_count.saturating_sub(1));
    (min_idx <= max_idx).then(|| max_idx.saturating_sub(min_idx).saturating_add(1))
}

fn visible_items_for_snapshot(snapshot: &crate::app::dual_panel::PanelSnapshot) -> &[FileEntry] {
    if snapshot.items_snapshot_compact && snapshot.items.is_empty() {
        snapshot.all_items.as_ref().as_slice()
    } else {
        snapshot.items.as_ref().as_slice()
    }
}

/// Inserts every path an item can be referenced by (its own path and its
/// folder cover, if any) into `set`.
fn insert_item_reference_paths(set: &mut FxHashSet<std::path::PathBuf>, item: &FileEntry) {
    set.insert(item.path.clone());
    if let Some(cover) = item.folder_cover.as_ref() {
        set.insert(cover.clone());
    }
}

/// Builds the set of every path the inactive panel snapshot can reference:
/// its selection (path + cover) and its visible items (path + cover).
///
/// Built as an O(n) one-shot so a burst of `m` stale results costs O(n+m)
/// instead of O(n*m) linear rescans.
fn collect_inactive_snapshot_paths(
    snapshot: &crate::app::dual_panel::PanelSnapshot,
) -> FxHashSet<std::path::PathBuf> {
    let mut set = FxHashSet::default();
    if let Some(selected) = snapshot.selected_file.as_ref() {
        insert_item_reference_paths(&mut set, selected);
    }
    for item in visible_items_for_snapshot(snapshot) {
        insert_item_reference_paths(&mut set, item);
    }
    set
}

fn insert_visible_paths_from_range(
    visible_paths: &mut FxHashSet<std::path::PathBuf>,
    items: &[FileEntry],
    visible_index_range: Option<(usize, usize)>,
) {
    let Some((min_idx, max_idx)) = visible_index_range else {
        return;
    };
    if items.is_empty() {
        return;
    }

    let max_idx = max_idx.min(items.len().saturating_sub(1));
    if min_idx > max_idx {
        return;
    }

    visible_paths.reserve(max_idx.saturating_sub(min_idx).saturating_add(1));
    for item in items.iter().skip(min_idx).take(max_idx - min_idx + 1) {
        visible_paths.insert(item.path.clone());
    }
}

impl ImageViewerApp {
    pub(crate) fn all_items_mut(&mut self) -> &mut Vec<FileEntry> {
        self.items_revision = self.items_revision.wrapping_add(1);
        Arc::make_mut(&mut self.all_items)
    }

    pub(crate) fn current_items_rebuild_signature(
        &self,
    ) -> crate::app::state::ItemsRebuildSignature {
        crate::app::state::ItemsRebuildSignature {
            items_revision: self.items_revision,
            search_query: self.search_query.clone(),
            active_tag_filter: self.active_tag_filter,
            tag_assignments_epoch: self.tag_assignments_epoch,
            sort_mode: self.sort_mode,
            sort_descending: self.sort_descending,
            folders_position: self.folders_position,
            group_mode: self.group_mode,
            group_descending: self.group_descending,
            path: self.navigation_state.current_path.clone(),
            is_computer_view: self.navigation_state.is_computer_view,
            is_recycle_bin_view: self.navigation_state.is_recycle_bin_view,
        }
    }

    pub(crate) fn share_visible_items_from_all_items(&mut self) {
        self.items = self.all_items.clone();
        self.total_items = self.items.len();
    }

    pub(crate) fn clear_pending_items_rebuild_flags(&mut self) {
        self.pending_items_rebuild = false;
        self.pending_items_count = 0;
    }

    pub(crate) fn invalidate_active_items_rebuild(&mut self) {
        self.items_rebuild_request_id = self.items_rebuild_request_id.wrapping_add(1);
        self.items_rebuild_in_flight = false;
        self.inactive_final_items_rebuild_pending = false;
        self.clear_pending_items_rebuild_flags();
        self.last_items_rebuild = Instant::now();
    }

    pub(crate) fn invalidate_and_schedule_items_rebuild(&mut self) {
        self.invalidate_active_items_rebuild();
        self.pending_items_rebuild = true;
        self.pending_items_count = usize::MAX;
    }

    pub(crate) fn should_preserve_inactive_dual_panel_thumbnail_pipeline(&self) -> bool {
        self.dual_panel_enabled
            && self
                .dual_panel_inactive_state
                .as_ref()
                .is_some_and(|snapshot| {
                    panel_thumbnail_caches_active(
                        snapshot.view_mode,
                        snapshot.is_computer_view,
                        snapshot.is_recycle_bin_view,
                        visible_items_for_snapshot(snapshot).len(),
                    )
                })
    }

    /// Builds a reusable snapshot of all paths owned by the inactive panel.
    ///
    /// Returns `None` when there is no inactive panel (dual panel disabled or no
    /// snapshot), in which case no stale result can belong to it. Callers build
    /// this lazily on the first result whose generation diverges and reuse it
    /// for the rest of the drain, turning O(n*m) rescans into O(n+m).
    pub(crate) fn collect_inactive_panel_paths(&self) -> Option<FxHashSet<std::path::PathBuf>> {
        if !self.dual_panel_enabled {
            return None;
        }
        let snapshot = self.dual_panel_inactive_state.as_ref()?;
        Some(collect_inactive_snapshot_paths(snapshot))
    }

    /// Returns `true` while the post-restore burst window is active.
    /// During burst, thumbnail upload throttling is bypassed to recover visual
    /// state quickly after the OS pages out the GPU working set.
    pub fn is_in_restore_burst(&self) -> bool {
        self.restore_burst_until
            .is_some_and(|deadline| Instant::now() < deadline)
    }

    /// Returns `true` when the active GPU backend is OpenGL-based.
    ///
    /// OpenGL uploads are synchronous on the CPU thread (each `ctx.load_texture`
    /// blocks until the driver finishes the transfer), unlike DX12/Vulkan where
    /// wgpu queues the upload asynchronously.  This method is used to apply more
    /// conservative per-frame upload limits that prevent UI freezes on OpenGL
    /// backends (Glow).
    pub fn is_opengl_backend(&self) -> bool {
        self.active_gpu_backend == "glow"
    }

    /// Returns `true` when the active wgpu backend is Vulkan.
    /// Vulkan has the best throughput in this app, but queued texture uploads can
    /// hold staging/RGBA memory longer than the generic wgpu path expects.
    pub fn is_vulkan_backend(&self) -> bool {
        self.active_gpu_backend == "Vulkan"
    }

    /// Returns `true` for asynchronous wgpu backends that should share the
    /// tighter thumbnail intake, upload, and RGBA-retention limits.
    pub fn uses_conservative_thumbnail_upload_policy(&self) -> bool {
        backend_uses_conservative_thumbnail_upload_policy(&self.active_gpu_backend)
    }

    /// Returns `true` for backends that need conservative folder-preview
    /// discovery, composition, and upload pacing.
    pub fn uses_conservative_folder_preview_policy(&self) -> bool {
        backend_uses_low_ram_gpu_policy(&self.active_gpu_backend)
    }

    /// Returns `true` for GPU backends that should use the low-RAM thumbnail
    /// cache profile and working-set trims.
    pub fn uses_aggressive_gpu_memory_policy(&self) -> bool {
        backend_uses_low_ram_gpu_policy(&self.active_gpu_backend)
    }

    /// Check if a video is actively playing in docked mode (preview panel)
    /// Used to throttle disk I/O from thumbnails to prevent stutter during video playback
    pub fn is_video_playing_docked(&self) -> bool {
        if let Some(preview) = &self.media_preview {
            // Must be: (1) docked (not detached), (2) visible/initialized, and (3) playing
            if !preview.is_detached() && preview.is_player_visible() {
                if let Some(state) = preview.get_video_state() {
                    return state.is_playing;
                }
            }
        }
        false
    }

    pub(crate) fn effective_thumbnail_request_size_px(&self, logical_size_px: u32) -> u32 {
        let scale = self.ui_ctx.pixels_per_point().max(1.0);
        (((logical_size_px.max(1) as f32) * scale).ceil() as u32)
            .min(crate::domain::thumbnail::MAX_THUMBNAIL_SIDE)
    }

    pub(crate) fn current_thumbnail_bucket_size(&self) -> u32 {
        let logical_size = self.thumbnail_size.max(crate::ui::theme::THUMBNAIL_MIN) as u32;
        get_bucket_size(self.effective_thumbnail_request_size_px(logical_size))
    }

    pub(crate) fn effective_folder_preview_request_size_px(&self) -> u32 {
        let scale = self.ui_ctx.pixels_per_point().max(1.0);
        let logical_size = self.thumbnail_size.max(crate::ui::theme::THUMBNAIL_MIN) * 0.85;
        let display_size = ((logical_size.max(1.0)) * scale).ceil() as u32;
        // Ensure at least bucket 512 to avoid re-extraction when zooming
        display_size.max(257)
    }

    pub(crate) fn current_folder_preview_bucket_size(&self) -> u32 {
        get_bucket_size(self.effective_folder_preview_request_size_px())
    }

    pub(crate) fn current_dynamic_texture_keep_count(&self) -> usize {
        if !self.thumbnail_caches_active() {
            return IDLE_THUMBNAIL_TEXTURE_KEEP;
        }

        let visible_items = self.visible_grid_items_for_cache();
        let target = dynamic_texture_keep_count(visible_items);
        if self.uses_aggressive_gpu_memory_policy() {
            let cap = LOW_RAM_GPU_MAX_DYNAMIC_TEXTURE_CACHE_ITEMS
                .max(visible_items)
                .min(MAX_DYNAMIC_TEXTURE_CACHE_ITEMS);
            target.min(cap).max(MIN_DYNAMIC_TEXTURE_CACHE_ITEMS)
        } else {
            target
        }
    }

    pub(crate) fn current_dynamic_folder_preview_keep_count(&self) -> usize {
        if !self.thumbnail_caches_active() {
            return IDLE_FOLDER_PREVIEW_KEEP;
        }

        let visible_items = self.visible_grid_items_for_cache();
        let target = if self.uses_aggressive_gpu_memory_policy() {
            // Low-RAM GPU profile: prefer releasing offscreen folders over
            // holding every folder in large directories.
            dynamic_texture_keep_count(visible_items)
        } else {
            dynamic_folder_preview_keep_count(visible_items, self.current_directory_folder_count())
        };
        if self.uses_aggressive_gpu_memory_policy() {
            let cap = LOW_RAM_GPU_MAX_DYNAMIC_FOLDER_PREVIEW_ITEMS
                .max(visible_items)
                .min(MAX_DYNAMIC_FOLDER_PREVIEW_ITEMS);
            target.min(cap).max(MIN_DYNAMIC_FOLDER_PREVIEW_ITEMS)
        } else {
            target
        }
    }

    pub(crate) fn current_dynamic_rgba_budget_bytes(&self, floor_bytes: usize) -> usize {
        if !self.thumbnail_caches_active() {
            return IDLE_RGBA_BUDGET_BYTES;
        }

        dynamic_rgba_budget_bytes(
            self.visible_grid_items_for_cache(),
            self.current_thumbnail_bucket_size(),
            floor_bytes,
        )
    }

    pub(crate) fn current_thumbnail_rgba_budget_bytes(&self) -> usize {
        let floor_bytes = if self.uses_aggressive_gpu_memory_policy() {
            LOW_RAM_GPU_RGBA_BUDGET_FLOOR_BYTES
        } else {
            DEFAULT_DYNAMIC_RGBA_BUDGET_BYTES
        };
        let budget = self.current_dynamic_rgba_budget_bytes(floor_bytes);

        if self.uses_aggressive_gpu_memory_policy() && self.thumbnail_caches_active() {
            budget.clamp(MIN_RGBA_BUDGET_BYTES, LOW_RAM_GPU_MAX_RGBA_BUDGET_BYTES)
        } else {
            budget
        }
    }

    pub(crate) fn current_pending_thumbnail_upload_byte_limit(&self) -> usize {
        let bucket_size = self.current_thumbnail_bucket_size() as usize;
        let bucket_bytes = bucket_size
            .saturating_mul(bucket_size)
            .saturating_mul(4)
            .max(1);

        if !self.thumbnail_caches_active() {
            return bucket_bytes
                .saturating_mul(IDLE_PENDING_THUMBNAILS)
                .max(MIN_RGBA_BUDGET_BYTES);
        }

        if self.uses_aggressive_gpu_memory_policy() {
            LOW_RAM_GPU_MAX_PENDING_THUMBNAIL_RGBA_BYTES
        } else {
            MAX_PENDING_THUMBNAIL_RGBA_BYTES
        }
    }

    pub(crate) fn current_pending_thumbnail_upload_limit(&self) -> usize {
        if !self.thumbnail_caches_active() {
            return IDLE_PENDING_THUMBNAILS;
        }

        let bucket_size = self.current_thumbnail_bucket_size() as usize;
        let bucket_bytes = bucket_size
            .saturating_mul(bucket_size)
            .saturating_mul(4)
            .max(1);
        let byte_limited_items = (self.current_pending_thumbnail_upload_byte_limit()
            / bucket_bytes)
            .max(MIN_DYNAMIC_PENDING_THUMBNAILS);

        self.current_dynamic_texture_keep_count()
            .clamp(BASE_PENDING_THUMBNAILS, MAX_DYNAMIC_PENDING_THUMBNAILS)
            .min(byte_limited_items)
    }

    pub(crate) fn pending_thumbnail_rgba_bytes(&self) -> usize {
        self.pending_thumbnails
            .iter()
            .map(|thumbnail| thumbnail.image_data.len())
            .sum()
    }

    pub(crate) fn trim_pending_thumbnail_uploads_to_limit(
        &mut self,
        pending_bytes: usize,
        visible_paths: Option<&FxHashSet<std::path::PathBuf>>,
    ) -> usize {
        let max_pending = self.current_pending_thumbnail_upload_limit();
        let max_pending_bytes = self.current_pending_thumbnail_upload_byte_limit();
        let selected_path = self.selected_file.as_ref().map(|file| file.path.clone());
        let (removed, final_pending_bytes) = trim_pending_thumbnail_queue(
            &mut self.pending_thumbnails,
            pending_bytes,
            max_pending,
            max_pending_bytes,
            visible_paths,
            selected_path.as_ref(),
        );
        for thumbnail in removed {
            self.cache_manager.finish_pending_upload(&thumbnail.path);
        }
        final_pending_bytes
    }

    pub fn log_memory_snapshot(&mut self, label: &str) {
        if !memory_trace_enabled() {
            return;
        }

        let Some(process) = current_process_memory_snapshot() else {
            return;
        };

        let pending_thumbnail_bytes: usize = self
            .pending_thumbnails
            .iter()
            .map(|thumbnail| thumbnail.image_data.len())
            .sum();
        let pending_thumbnail_limit = self.current_pending_thumbnail_upload_limit();
        let pending_thumbnail_byte_limit = self.current_pending_thumbnail_upload_byte_limit();
        let (directory_cache_folders, directory_cache_items) = self.directory_cache.stats();
        let (gif_entries, gif_bytes) = self.gif_manager.stats();
        let (
            icon_items,
            extension_icon_items,
            drive_icon_items,
            failed_drive_icons,
            loading_drive_icons,
        ) = self.item_icon_loader.cache_counts();
        let texture_items = self.cache_manager.texture_cache.len();
        let texture_cap = self.cache_manager.texture_cache.cap().get();
        let folder_preview_items = self.cache_manager.folder_preview_cache.len();
        let folder_preview_cap = self.cache_manager.folder_preview_cache.cap().get();
        let rgba_items = self.cache_manager.rgba_data_cache.len();
        let rgba_bytes = self.cache_manager.estimate_ram_cache_usage();
        let vram_estimate = self.cache_manager.estimate_vram_usage();
        let visible_grid_items = self.visible_grid_items_for_cache();
        let texture_target = self.current_dynamic_texture_keep_count();
        let folder_preview_target = self.current_dynamic_folder_preview_keep_count();
        let rgba_target = self.current_thumbnail_rgba_budget_bytes();

        // Extra diagnostics — coleções não cobertas pelos campos principais.
        // Mantidas em variáveis locais para evitar custo se MTT_MEMORY_TRACE estiver off
        // (chamador já gateia via memory_trace_enabled()).
        let fs_size_cache = self.folder_size_state.cache.len();
        let fs_size_loading = self.folder_size_state.loading.len();
        let fs_batch_cache = self.folder_size_state.batch_cache.len();
        let fs_batch_loading = self.folder_size_state.batch_loading.len();
        let fs_pending_reval = self.folder_size_state.pending_revalidation.len();
        let fs_inval_epoch = self.folder_size_state.batch_invalidation_epoch.len();
        let live_size_cache = self.live_file_size_cache.len();
        let live_size_loading = self.live_file_size_loading.len();
        let metadata_cache_n = self.metadata_cache.len();
        let metadata_loading_n = self.metadata_loading.len();
        let scanned_folders_n = self.scanned_folders.len();
        let failed_icons_n = self.failed_icons.len();
        let loading_icons_n = self.loading_icons.len();
        let deletion_date_cache_n = self.deletion_date_cache.len();
        let visible_paths_cache_n = self.visible_paths_cache.len();
        let pending_mtime_recheck_n = self.pending_folder_mtime_recheck.len();
        let multi_selection_n = self.multi_selection.len();
        let drag_payload_n = self.drag_payload_paths.len();
        let pinned_n = self.pinned_folders.len();
        let dirty_registry_n = self.directory_dirty_registry.len();
        let request_epochs_n = self.thumbnail_request_epochs.len();
        let attempted_bucket_n = self.cache_manager.attempted_thumbnail_bucket.len();
        let folder_preview_trace = self.cache_manager.folder_preview_trace.take_snapshot();
        let thumbnail_trace = self.cache_manager.thumbnail_trace.take_snapshot();

        log::info!(
            "[MEM-TRACE:{label}] backend={} ws={:.1}MB private={:.1}MB items={} all_items={} tabs={} dir_cache={}/{} visible_items={} textures={}/{} texture_target={} folder_tex={}/{} folder_target={} rgba_items={} rgba={:.1}/{:.1}MB pending={}/{} pending_rgba={:.1}/{:.1}MB pending_set={} loading={} folder_loading={} failed_thumbs={} queue={} img_rx={} vram_est={:.1}MB icons={} ext_icons={} drive_icons={} failed_drive_icons={} loading_drive_icons={} gifs={} gif_rgba={:.1}MB visible={:?} thumb_bucket={} folder_bucket={} frame_avg={:.1}ms frame_peak={:.1}ms upload_budget={:.1}ms request_epochs={} attempted_bucket={} fs_size={}/{} fs_batch={}/{} fs_reval={} fs_inval_ep={} live_size={}/{} meta={}/{} scanned={} failed_ico={} loading_ico={} del_date={} vis_paths={} mtime_re={} multisel={} drag={} pinned={} dirty_reg={} fp_req={} fp_dup={} fp_dbnc={} fp_inval={} fp_upl={} fp_upl_none={} fp_upl_diff={} fp_evict={} fp_db_w={} fp_comp={} fp_sample={:?} th_req={} th_dupL={} th_dupP={} th_pdel={} th_ram={} th_disp={} th_upl={} th_upl_dup={} th_evict={} th_uniq={} th_top={:?} th_req_sample={:?} th_upl_sample={:?}",
            self.active_gpu_backend.as_str(),
            bytes_to_mb(process.working_set_bytes),
            bytes_to_mb(process.private_usage_bytes),
            self.items.len(),
            self.all_items.len(),
            self.tab_manager.count(),
            directory_cache_folders,
            directory_cache_items,
            visible_grid_items,
            texture_items,
            texture_cap,
            texture_target,
            folder_preview_items,
            folder_preview_cap,
            folder_preview_target,
            rgba_items,
            bytes_to_mb(rgba_bytes as u64),
            bytes_to_mb(rgba_target as u64),
            self.pending_thumbnails.len(),
            pending_thumbnail_limit,
            bytes_to_mb(pending_thumbnail_bytes as u64),
            bytes_to_mb(pending_thumbnail_byte_limit as u64),
            self.cache_manager.pending_upload_set.len(),
            self.cache_manager.loading_set.len(),
            self.cache_manager.folder_preview_loading.len(),
            self.cache_manager.failed_thumbnails.len(),
            self.thumbnail_queue.pending_count(),
            self.image_receiver.len(),
            bytes_to_mb(vram_estimate as u64),
            icon_items,
            extension_icon_items,
            drive_icon_items,
            failed_drive_icons,
            loading_drive_icons,
            gif_entries,
            bytes_to_mb(gif_bytes as u64),
            self.visible_index_range,
            self.current_thumbnail_bucket_size(),
            self.current_folder_preview_bucket_size(),
            self.frame_time_avg_ms,
            self.frame_time_peak_ms,
            self.upload_budget_ms,
            request_epochs_n,
            attempted_bucket_n,
            fs_size_cache,
            fs_size_loading,
            fs_batch_cache,
            fs_batch_loading,
            fs_pending_reval,
            fs_inval_epoch,
            live_size_cache,
            live_size_loading,
            metadata_cache_n,
            metadata_loading_n,
            scanned_folders_n,
            failed_icons_n,
            loading_icons_n,
            deletion_date_cache_n,
            visible_paths_cache_n,
            pending_mtime_recheck_n,
            multi_selection_n,
            drag_payload_n,
            pinned_n,
            dirty_registry_n,
            folder_preview_trace.requests,
            folder_preview_trace.duplicate_skips,
            folder_preview_trace.debounce_skips,
            folder_preview_trace.invalidations,
            folder_preview_trace.uploads,
            folder_preview_trace.upload_no_cache,
            folder_preview_trace.upload_size_diff,
            folder_preview_trace.lru_evictions,
            folder_preview_trace.db_writes,
            folder_preview_trace.composes,
            folder_preview_trace.sample_path,
            thumbnail_trace.req_total,
            thumbnail_trace.req_dup_loading,
            thumbnail_trace.req_dup_pending,
            thumbnail_trace.req_pending_deletion,
            thumbnail_trace.ram_cache_hit,
            thumbnail_trace.worker_dispatch,
            thumbnail_trace.uploads,
            thumbnail_trace.upload_already_cached,
            thumbnail_trace.upload_evictions,
            thumbnail_trace.unique_request_paths,
            thumbnail_trace.top_paths,
            thumbnail_trace.sample_request_path,
            thumbnail_trace.sample_upload_path,
        );
    }

    pub fn maybe_log_memory_snapshot(&mut self, label: &str) {
        if !memory_trace_enabled() || self.last_memory_trace_log.elapsed() < MEMORY_TRACE_INTERVAL {
            return;
        }

        self.last_memory_trace_log = Instant::now();
        self.log_memory_snapshot(label);
    }

    /// Check if the media player should currently capture all keyboard arrow/space input.
    /// Returns true if player is detached/fullscreen AND has focus.
    pub fn is_media_keyboard_focused(&self) -> bool {
        let preview = if let Some(p) = &self.media_preview {
            p
        } else {
            return false;
        };

        // Condition 1: Must be detached or fullscreen
        if !preview.is_detached() && !preview.is_maximized() {
            return false;
        }

        // Condition 2: Current tab must be the owner
        let active_tab_id = self.tab_manager.active().id;
        if self.media_preview_owner_tab_id != Some(active_tab_id) {
            return false;
        }

        #[cfg(target_os = "windows")]
        {
            use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
            let foreground = unsafe { GetForegroundWindow() };
            if foreground.is_invalid() {
                return false;
            }

            // Focused if either the main app or the MPV child window is in foreground
            self.native_hwnd == Some(foreground) || preview.get_hwnd() == Some(foreground)
        }
        #[cfg(not(target_os = "windows"))]
        {
            false
        }
    }

    /// Applies bounded cache cleanup when process memory is above thresholds.
    /// Keeps hot assets while avoiding long-session RAM growth.
    pub fn run_memory_maintenance(&mut self) {
        self.run_memory_maintenance_impl(false);
    }

    /// Runs memory maintenance immediately, bypassing normal periodic throttle.
    pub fn run_memory_maintenance_now(&mut self) {
        self.run_memory_maintenance_impl(true);
    }

    fn working_set_trim_blocker_reason(&self) -> Option<&'static str> {
        let video_playing = self
            .media_preview
            .as_ref()
            .and_then(|preview| preview.get_video_state())
            .is_some_and(|state| state.is_playing);
        let purge_running = self
            .purge_worker_state
            .as_ref()
            .is_some_and(|state| state.running.load(Ordering::Relaxed));
        let thumbnail_pipeline_busy = self.thumbnail_queue.pending_count() > 0
            || !self.pending_thumbnails.is_empty()
            || !self.image_receiver.is_empty()
            || !self.cache_manager.loading_set.is_empty()
            || !self.cache_manager.folder_preview_loading.is_empty()
            || !self.cache_manager.pending_upload_set.is_empty();
        let compression_active = self
            .file_operation_state
            .compression_progress
            .lock()
            .is_ok_and(|guard| guard.is_some());
        let inactive_folder_loading = self
            .dual_panel_inactive_state
            .as_ref()
            .is_some_and(|snapshot| snapshot.is_loading_folder);

        if self.is_in_restore_burst() {
            Some("restore-burst")
        } else if self.last_user_activity.elapsed() < WORKING_SET_TRIM_ACTIVITY_GRACE {
            Some("recent-user-activity")
        } else if video_playing {
            Some("video-playing")
        } else if self.file_operations_active_for_background() {
            Some("file-operation")
        } else if self.is_loading_folder {
            Some("folder-loading")
        } else if inactive_folder_loading {
            Some("inactive-folder-loading")
        } else if self.items_rebuild_in_flight {
            Some("items-rebuild")
        } else if !self.inactive_items_rebuild_registry.is_empty() {
            Some("inactive-items-rebuild")
        } else if self.bulk_thumbnail_scanning.load(Ordering::Relaxed) {
            Some("bulk-thumbnail-scan")
        } else if !self.file_hash_loading.is_empty() {
            Some("file-hash")
        } else if !self.folder_size_state.loading.is_empty() {
            Some("folder-size")
        } else if !self.folder_size_state.batch_loading.is_empty() {
            Some("folder-size-batch")
        } else if !self.metadata_loading.is_empty() {
            Some("metadata")
        } else if !self.live_file_size_loading.is_empty() {
            Some("live-file-size")
        } else if self.global_search.in_flight_started_at.is_some() {
            Some("global-search")
        } else if purge_running {
            Some("tag-purge")
        } else if self.shell_menu_loading {
            Some("shell-menu")
        } else if self.open_with_loading {
            Some("open-with")
        } else if compression_active {
            Some("compression")
        } else if thumbnail_pipeline_busy {
            Some("thumbnail-pipeline")
        } else if !self.loading_icons.is_empty() || !self.loading_extensions.is_empty() {
            Some("icon-pipeline")
        } else if self.item_icon_loader.has_pending_auxiliary_icon_work() {
            Some("auxiliary-icon-pipeline")
        } else {
            None
        }
    }

    fn working_set_trim_blocked(&self) -> bool {
        self.working_set_trim_blocker_reason().is_some()
    }

    pub(crate) fn file_operations_active_for_background(&self) -> bool {
        self.file_operation_state.file_ops_in_progress > 0
    }

    pub(crate) fn refresh_working_set_trim_blocker(&self, force_blocked: bool) {
        let blocker_reason = if force_blocked {
            Some("window-size-move")
        } else {
            self.working_set_trim_blocker_reason()
        };
        let blocked = blocker_reason.is_some();
        let was_blocked = WORKING_SET_TRIM_BLOCKED.swap(blocked, Ordering::AcqRel);
        if blocked && !was_blocked {
            WORKING_SET_TRIM_EPOCH.fetch_add(1, Ordering::AcqRel);
            let reason = blocker_reason.unwrap_or("unknown");
            log::debug!("[MEMORY] working-set trim blocked by {reason}");
            crate::infrastructure::diagnostic_logger::diag_info(
                "memory_trim",
                "blocked",
                &[crate::infrastructure::diagnostic_logger::field_label(
                    "reason", reason,
                )],
            );
        } else if !blocked && was_blocked {
            crate::infrastructure::diagnostic_logger::diag_info("memory_trim", "unblocked", &[]);
            if self.uses_aggressive_gpu_memory_policy()
                && (self.background_memory_trim_active
                    || self.last_restore_time.elapsed() >= WORKING_SET_TRIM_MIN_INTERVAL)
                && current_process_memory_snapshot().is_some_and(|snapshot| {
                    snapshot.working_set_bytes >= LOW_RAM_GPU_IDLE_WS_TRIM_MIN_BYTES
                })
            {
                request_process_working_set_trim_series(
                    format!("gpu activity completed backend={}", self.active_gpu_backend),
                    WORKING_SET_TRIM_FOLLOW_UP_DELAYS,
                    false,
                );
            }
        }
    }

    pub(crate) fn begin_background_memory_trim(&mut self) {
        self.background_memory_trim_active = true;
        self.background_memory_trim_pending = true;
        self.background_memory_trim_last_stable_working_set_bytes = 0;
        self.release_thumbnail_memory_for_background();
        self.background_memory_trim_success_baseline =
            WORKING_SET_TRIM_SUCCESS_COUNT.load(Ordering::Acquire);
        self.refresh_working_set_trim_blocker(false);
        if process_working_set_trim_disabled() {
            self.background_memory_trim_active = false;
            self.background_memory_trim_pending = false;
            crate::infrastructure::diagnostic_logger::diag_info(
                "memory_trim",
                "background_trim_disabled",
                &[],
            );
            return;
        }
        crate::infrastructure::diagnostic_logger::diag_info(
            "memory_trim",
            "background_trim_armed",
            &[],
        );
    }

    pub(crate) fn cancel_background_memory_trim(&mut self) {
        let _execution_guard = working_set_trim_execution_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.background_memory_trim_pending {
            self.background_memory_trim_pending = false;
            let event = if WORKING_SET_TRIM_SUCCESS_COUNT.load(Ordering::Acquire)
                > self.background_memory_trim_success_baseline
            {
                "background_trim_completed_before_restore"
            } else {
                "background_trim_cancelled_on_restore"
            };
            crate::infrastructure::diagnostic_logger::diag_info("memory_trim", event, &[]);
        }
        self.background_memory_trim_active = false;
        self.background_memory_trim_pending = false;
        self.background_memory_trim_last_stable_working_set_bytes = 0;
        WORKING_SET_TRIM_EPOCH.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn run_background_memory_maintenance(&mut self) {
        if !self.background_memory_trim_active
            || crate::infrastructure::windows::window_subclass::layout_phase()
                != crate::infrastructure::windows::window_subclass::WindowLayoutPhase::Minimized
        {
            return;
        }
        self.discard_background_thumbnail_results();
        self.discard_icon_worker_results_for_background();
        self.item_icon_loader
            .discard_async_icon_results_for_background();
        self.refresh_working_set_trim_blocker(false);

        let success_count = WORKING_SET_TRIM_SUCCESS_COUNT.load(Ordering::Acquire);
        if success_count > self.background_memory_trim_success_baseline {
            let completed_pending_trim = self.background_memory_trim_pending;
            self.background_memory_trim_pending = false;
            self.background_memory_trim_success_baseline = success_count;
            self.background_memory_trim_last_stable_working_set_bytes =
                LAST_EFFECTIVE_WORKING_SET_TRIM_BYTES.load(Ordering::Acquire);
            crate::infrastructure::diagnostic_logger::diag_info(
                "memory_trim",
                if completed_pending_trim {
                    "background_trim_completed"
                } else {
                    "background_trim_follow_up_completed"
                },
                &[],
            );
        }

        if self.last_memory_maintenance.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_memory_maintenance = Instant::now();
        self.refresh_working_set_trim_blocker(false);

        if self.working_set_trim_blocked() {
            return;
        }

        let Some(snapshot) = current_process_memory_snapshot() else {
            return;
        };
        if snapshot.working_set_bytes < LOW_RAM_GPU_IDLE_WS_TRIM_MIN_BYTES {
            if self.background_memory_trim_pending {
                self.background_memory_trim_pending = false;
                self.background_memory_trim_success_baseline = success_count;
                crate::infrastructure::diagnostic_logger::diag_info(
                    "memory_trim",
                    "background_trim_not_needed",
                    &[],
                );
            }
            self.background_memory_trim_last_stable_working_set_bytes = snapshot.working_set_bytes;
            return;
        }

        if background_trim_should_rearm(
            self.background_memory_trim_active,
            self.background_memory_trim_pending,
            snapshot.working_set_bytes,
            self.background_memory_trim_last_stable_working_set_bytes,
        ) {
            self.background_memory_trim_pending = true;
            self.background_memory_trim_success_baseline = success_count;
            crate::infrastructure::diagnostic_logger::diag_info(
                "memory_trim",
                "background_trim_rearmed",
                &[],
            );
        }

        if self.background_memory_trim_pending
            && request_process_working_set_trim_series(
                format!(
                    "window minimized backend={} ws={:.1}MB",
                    self.active_gpu_backend,
                    bytes_to_mb(snapshot.working_set_bytes)
                ),
                WORKING_SET_TRIM_FOLLOW_UP_DELAYS,
                true,
            )
        {
            crate::infrastructure::diagnostic_logger::diag_info(
                "memory_trim",
                "background_trim_scheduled",
                &[],
            );
        }
    }

    fn discard_background_thumbnail_results(&mut self) {
        while let Ok(thumbnail_data) = self.image_receiver.try_recv() {
            self.cache_manager.finish_loading(&thumbnail_data.path);
            self.cache_manager
                .finish_pending_upload(&thumbnail_data.path);
        }
        while let Ok(preview_data) = self.folder_preview_receiver.try_recv() {
            self.cache_manager
                .finish_folder_preview_loading(&preview_data.path);
        }
        self.pending_thumbnails.clear();
        self.cache_manager.pending_upload_set.clear();
    }

    fn release_thumbnail_memory_for_background(&mut self) {
        let loading_paths: Vec<_> = self.cache_manager.loading_set.iter().cloned().collect();
        for path in self.thumbnail_queue.cancel_normal_paths(&loading_paths) {
            self.cache_manager.finish_loading(&path);
        }
        self.discard_background_thumbnail_results();
        self.pending_thumbnails.clear();
        self.pending_thumbnails.shrink_to_fit();
        self.cache_manager.pending_upload_set.clear();

        let (textures, rgba, folder_previews, rgba_bytes) = self
            .cache_manager
            .release_thumbnail_cache_storage_for_background(
                INACTIVE_THUMBNAIL_CACHE_ITEMS,
                INACTIVE_THUMBNAIL_CACHE_ITEMS,
                INACTIVE_THUMBNAIL_CACHE_ITEMS,
                0,
            );
        let (icons, extension_icons) = self.item_icon_loader.trim_icon_caches(128, 128);
        self.last_texture_cache_retune = Instant::now()
            .checked_sub(Duration::from_secs(10))
            .unwrap_or_else(Instant::now);

        log::debug!(
            "[MEMORY] background cache release: textures={} folder_previews={} rgba_items={} rgba={:.1}MB icons={} ext_icons={}",
            textures,
            folder_previews,
            rgba,
            bytes_to_mb(rgba_bytes as u64),
            icons,
            extension_icons,
        );
    }

    /// Drops stale visible thumbnail work and aggressively downsizes thumbnail
    /// caches when the visible folder/view changes. This is intentionally
    /// separate from memory-pressure maintenance: stale thumbnail textures and
    /// queued RGBA payloads from the previous folder should be released even
    /// when total process RAM is below the soft limit.
    pub(crate) fn discard_thumbnail_pipeline_for_navigation(
        &mut self,
        reason: &str,
        trim_icons: bool,
    ) {
        let queued_removed = self.thumbnail_queue.clear_pending();

        let mut receiver_drained = 0usize;
        while let Ok(thumbnail_data) = self.image_receiver.try_recv() {
            self.cache_manager.finish_loading(&thumbnail_data.path);
            self.cache_manager
                .finish_pending_upload(&thumbnail_data.path);
            receiver_drained += 1;
        }

        let mut folder_preview_receiver_drained = 0usize;
        while let Ok(preview_data) = self.folder_preview_receiver.try_recv() {
            self.cache_manager
                .finish_folder_preview_loading(&preview_data.path);
            folder_preview_receiver_drained += 1;
        }

        self.cache_manager.loading_set.clear();
        self.cache_manager.folder_preview_loading.clear();
        self.cache_manager.pending_upload_set.clear();
        self.cache_manager.attempted_thumbnail_bucket.clear();
        self.pending_folder_preview_replace.clear();
        self.suppress_next_folder_preview_invalidation.clear();
        self.pending_thumbnails.clear();
        self.pending_thumbnails.shrink_to_fit();
        self.thumbnail_request_epochs.clear();
        self.thumbnail_request_epochs.shrink_to_fit();

        let old_textures = self.cache_manager.texture_cache.len();
        let old_texture_cap = self.cache_manager.texture_cache.cap().get();
        let old_folder_previews = self.cache_manager.folder_preview_cache.len();
        let old_folder_preview_cap = self.cache_manager.folder_preview_cache.cap().get();
        let old_rgba_bytes = self.cache_manager.estimate_ram_cache_usage();

        let (released_textures, released_rgba, released_folder_previews, released_rgba_bytes) =
            if self.uses_aggressive_gpu_memory_policy() {
                self.cache_manager.release_thumbnail_caches_for_idle(
                    INACTIVE_THUMBNAIL_CACHE_ITEMS,
                    INACTIVE_THUMBNAIL_CACHE_ITEMS,
                    INACTIVE_THUMBNAIL_CACHE_ITEMS,
                    0,
                )
            } else {
                self.cache_manager
                    .retune_texture_cache_capacity(MIN_DYNAMIC_TEXTURE_CACHE_ITEMS);
                self.cache_manager
                    .retune_folder_preview_cache_capacity(MIN_DYNAMIC_FOLDER_PREVIEW_ITEMS);
                self.cache_manager
                    .retune_rgba_cache_capacity(NAVIGATION_RGBA_CACHE_ITEMS);
                self.cache_manager.retune_rgba_budget(MIN_RGBA_BUDGET_BYTES);
                let (textures_removed, rgba_removed, folder_previews_removed) =
                    self.cache_manager.trim_thumbnail_caches(
                        MIN_DYNAMIC_TEXTURE_CACHE_ITEMS,
                        MIN_RGBA_BUDGET_BYTES,
                        MIN_DYNAMIC_FOLDER_PREVIEW_ITEMS,
                        None,
                    );
                (textures_removed, rgba_removed, folder_previews_removed, 0)
            };
        let (icon_evicted, ext_icon_evicted) = if trim_icons {
            self.item_icon_loader.trim_icon_caches(128, 128)
        } else {
            (0, 0)
        };

        self.last_texture_cache_retune = Instant::now()
            .checked_sub(Duration::from_secs(10))
            .unwrap_or_else(Instant::now);
        self.ui_ctx.request_repaint();

        if old_textures > MIN_DYNAMIC_TEXTURE_CACHE_ITEMS
            || old_folder_previews > MIN_DYNAMIC_FOLDER_PREVIEW_ITEMS
            || old_rgba_bytes > MIN_RGBA_BUDGET_BYTES
            || queued_removed > 0
            || receiver_drained > 0
            || folder_preview_receiver_drained > 0
            || icon_evicted > 0
            || ext_icon_evicted > 0
        {
            log::debug!(
                "[MEMORY] navigation trim reason={} backend={} textures={}/{} released_textures={} folder_previews={}/{} released_folder_previews={} rgba={:.1}MB released_rgba_items={} released_rgba={:.1}MB queued={} receiver={} fp_receiver={} icons={} ext_icons={}",
                reason,
                self.active_gpu_backend,
                old_textures,
                old_texture_cap,
                released_textures,
                old_folder_previews,
                old_folder_preview_cap,
                released_folder_previews,
                old_rgba_bytes as f64 / 1024.0 / 1024.0,
                released_rgba,
                released_rgba_bytes as f64 / 1024.0 / 1024.0,
                queued_removed,
                receiver_drained,
                folder_preview_receiver_drained,
                icon_evicted,
                ext_icon_evicted,
            );

            if self.uses_aggressive_gpu_memory_policy() {
                request_process_working_set_trim_series(
                    format!(
                        "gpu thumbnail navigation backend={} ({reason})",
                        self.active_gpu_backend
                    ),
                    WORKING_SET_TRIM_FOLLOW_UP_DELAYS,
                    false,
                );
            }
        }
    }

    pub(crate) fn prune_thumbnail_pipeline_for_dual_panel_navigation(&mut self, reason: &str) {
        let Some(visible_paths) = self.inactive_panel_visible_paths_snapshot() else {
            log::debug!(
                "[MEMORY] dual-panel navigation preserved inactive pipeline reason={} visible_paths=0",
                reason
            );
            return;
        };

        let queued_removed = self
            .thumbnail_queue
            .clear_pending_except_paths(&visible_paths);

        let pending_before = self.pending_thumbnails.len();
        self.pending_thumbnails
            .retain(|thumbnail| visible_paths.contains(&thumbnail.path));
        let pending_removed = pending_before.saturating_sub(self.pending_thumbnails.len());

        let loading_before = self.cache_manager.loading_set.len();
        self.cache_manager.loading_set.clear();
        let loading_removed = loading_before;

        let pending_upload_before = self.cache_manager.pending_upload_set.len();
        self.cache_manager
            .pending_upload_set
            .retain(|path| visible_paths.contains(path));
        let pending_upload_removed =
            pending_upload_before.saturating_sub(self.cache_manager.pending_upload_set.len());

        let folder_loading_before = self.cache_manager.folder_preview_loading.len();
        self.cache_manager
            .folder_preview_loading
            .retain(|path| visible_paths.contains(path));
        let folder_loading_removed =
            folder_loading_before.saturating_sub(self.cache_manager.folder_preview_loading.len());

        self.pending_folder_preview_replace
            .retain(|path| visible_paths.contains(path));
        self.suppress_next_folder_preview_invalidation
            .retain(|path| visible_paths.contains(path));
        self.thumbnail_request_epochs
            .retain(|path, _| visible_paths.contains(path));
        #[allow(clippy::filter_map_bool_then)]
        let scanned_paths_to_remove: Vec<_> = self
            .scanned_folders
            .iter()
            .filter_map(|(path, _)| (!visible_paths.contains(path)).then(|| path.clone()))
            .collect();
        for path in scanned_paths_to_remove {
            self.scanned_folders.pop(&path);
        }
        self.loading_icons
            .retain(|path| visible_paths.contains(path));
        self.loading_extensions.clear();
        self.failed_extensions.clear();

        self.cache_manager.promote_visible(&visible_paths);
        let texture_keep = self.current_dynamic_texture_keep_count();
        if self.cache_manager.texture_cache.cap().get() < texture_keep {
            self.cache_manager
                .retune_texture_cache_capacity(texture_keep);
        }

        let folder_preview_keep = self.current_dynamic_folder_preview_keep_count();
        if self.cache_manager.folder_preview_cache.cap().get() < folder_preview_keep {
            self.cache_manager
                .retune_folder_preview_cache_capacity(folder_preview_keep);
        }

        self.last_texture_cache_retune = Instant::now()
            .checked_sub(Duration::from_secs(10))
            .unwrap_or_else(Instant::now);

        if queued_removed > 0
            || pending_removed > 0
            || loading_removed > 0
            || pending_upload_removed > 0
            || folder_loading_removed > 0
        {
            log::debug!(
                "[MEMORY] dual-panel navigation prune reason={} preserved={} queued={} pending={} loading={} pending_upload={} folder_loading={}",
                reason,
                visible_paths.len(),
                queued_removed,
                pending_removed,
                loading_removed,
                pending_upload_removed,
                folder_loading_removed,
            );
        }
    }

    fn inactive_panel_visible_paths_snapshot(&self) -> Option<FxHashSet<std::path::PathBuf>> {
        let snapshot = self.dual_panel_inactive_state.as_ref()?;
        let mut visible_paths = FxHashSet::default();

        if matches!(
            snapshot.view_mode,
            ViewMode::Grid | ViewMode::List | ViewMode::ColumnList | ViewMode::Miller
        ) {
            if crate::application::grouping::is_grouping_rendered(
                snapshot.view_mode,
                &snapshot.group_projection,
            ) {
                visible_paths.extend(snapshot.visible_group_paths.iter().cloned());
            } else {
                insert_visible_paths_from_range(
                    &mut visible_paths,
                    visible_items_for_snapshot(snapshot),
                    snapshot.visible_index_range,
                );
            }
        }

        if let Some(selected) = snapshot.selected_file.as_ref() {
            visible_paths.insert(selected.path.clone());
            if let Some(cover) = selected.folder_cover.as_ref() {
                visible_paths.insert(cover.clone());
            }
        }

        (!visible_paths.is_empty()).then_some(visible_paths)
    }

    /// Fully releases thumbnail memory when the destination view cannot render
    /// thumbnails at all. This is intentionally stronger than navigation trim:
    /// no warm thumbnail/folder-preview/RGBA cache is useful in This PC or the
    /// Recycle Bin, and keeping those LRUs alive makes Task Manager memory look
    /// permanently elevated after browsing media-heavy folders.
    pub(crate) fn release_thumbnail_pipeline_for_inactive_view(
        &mut self,
        reason: &str,
        trim_icons: bool,
    ) {
        let queued_removed = self.thumbnail_queue.clear_pending();

        let mut receiver_drained = 0usize;
        let mut receiver_rgba_bytes = 0usize;
        while let Ok(thumbnail_data) = self.image_receiver.try_recv() {
            receiver_rgba_bytes =
                receiver_rgba_bytes.saturating_add(thumbnail_data.image_data.len());
            self.cache_manager.finish_loading(&thumbnail_data.path);
            self.cache_manager
                .finish_pending_upload(&thumbnail_data.path);
            receiver_drained += 1;
        }

        let mut folder_preview_receiver_drained = 0usize;
        let mut folder_preview_rgba_bytes = 0usize;
        while let Ok(preview_data) = self.folder_preview_receiver.try_recv() {
            folder_preview_rgba_bytes =
                folder_preview_rgba_bytes.saturating_add(preview_data.rgba_data.len());
            self.cache_manager
                .finish_folder_preview_loading(&preview_data.path);
            folder_preview_receiver_drained += 1;
        }

        let pending_removed = self.pending_thumbnails.len();
        let pending_rgba_bytes = self.pending_thumbnail_rgba_bytes();
        self.pending_thumbnails.clear();
        self.pending_thumbnails.shrink_to_fit();

        self.thumbnail_request_epochs.clear();
        self.thumbnail_request_epochs.shrink_to_fit();
        self.pending_folder_preview_replace.clear();
        self.pending_folder_preview_replace.shrink_to_fit();
        self.suppress_next_folder_preview_invalidation.clear();
        self.suppress_next_folder_preview_invalidation
            .shrink_to_fit();
        self.selected_thumbnail = None;

        let old_texture_cap = self.cache_manager.texture_cache.cap().get();
        let old_folder_preview_cap = self.cache_manager.folder_preview_cache.cap().get();
        let (textures_removed, rgba_removed, folder_previews_removed, rgba_bytes_removed) =
            self.cache_manager.release_thumbnail_caches_for_idle(
                INACTIVE_THUMBNAIL_CACHE_ITEMS,
                INACTIVE_THUMBNAIL_CACHE_ITEMS,
                INACTIVE_THUMBNAIL_CACHE_ITEMS,
                0,
            );

        let (icon_evicted, ext_icon_evicted) = if trim_icons {
            self.item_icon_loader.trim_icon_caches(128, 128)
        } else {
            (0, 0)
        };

        self.last_texture_cache_retune = Instant::now()
            .checked_sub(Duration::from_secs(10))
            .unwrap_or_else(Instant::now);
        self.ui_ctx.request_repaint();

        let released_rgba_bytes = rgba_bytes_removed
            .saturating_add(pending_rgba_bytes)
            .saturating_add(receiver_rgba_bytes)
            .saturating_add(folder_preview_rgba_bytes);
        let released_any = textures_removed > 0
            || folder_previews_removed > 0
            || rgba_removed > 0
            || released_rgba_bytes > 0
            || pending_removed > 0
            || queued_removed > 0
            || receiver_drained > 0
            || folder_preview_receiver_drained > 0
            || icon_evicted > 0
            || ext_icon_evicted > 0;

        if released_any {
            log::debug!(
                "[MEMORY] inactive thumbnail release reason={} textures={}/{} folder_previews={}/{} rgba_items={} rgba={:.1}MB pending={} pending_rgba={:.1}MB queued={} receiver={} receiver_rgba={:.1}MB fp_receiver={} fp_receiver_rgba={:.1}MB icons={} ext_icons={}",
                reason,
                textures_removed,
                old_texture_cap,
                folder_previews_removed,
                old_folder_preview_cap,
                rgba_removed,
                rgba_bytes_removed as f64 / 1024.0 / 1024.0,
                pending_removed,
                pending_rgba_bytes as f64 / 1024.0 / 1024.0,
                queued_removed,
                receiver_drained,
                receiver_rgba_bytes as f64 / 1024.0 / 1024.0,
                folder_preview_receiver_drained,
                folder_preview_rgba_bytes as f64 / 1024.0 / 1024.0,
                icon_evicted,
                ext_icon_evicted,
            );

            request_process_working_set_trim_series(
                format!("thumbnail inactive view ({reason})"),
                WORKING_SET_TRIM_FOLLOW_UP_DELAYS,
                false,
            );
        }
    }

    fn run_memory_maintenance_impl(&mut self, force: bool) {
        if !force && self.last_memory_maintenance.elapsed() < Duration::from_secs(2) {
            self.refresh_working_set_trim_blocker(false);
            return;
        }
        self.last_memory_maintenance = Instant::now();
        let working_set_trim_blocked = self.working_set_trim_blocked();
        self.refresh_working_set_trim_blocker(false);

        let thumbnails_active = self.thumbnail_caches_active();
        if !thumbnails_active && !self.is_in_restore_burst() {
            self.release_thumbnail_pipeline_for_inactive_view("inactive-maintenance", false);
        }

        let Some(process_memory) = current_process_memory_snapshot() else {
            return;
        };
        let working_set_bytes = process_memory.working_set_bytes;
        self.run_gpu_idle_working_set_trim(working_set_bytes, working_set_trim_blocked);

        // Proactive cache trim: even below the soft memory limit, excess
        // texture/RAM cache entries from a previous folder should not linger
        // indefinitely.  When the cache is much larger than the current
        // visible grid requires, trim it down to a modest overshoot (2×)
        // so memory is released promptly after navigation.
        if !self.is_in_restore_burst() && thumbnails_active {
            let texture_keep = self.current_dynamic_texture_keep_count();
            let texture_count = self.cache_manager.texture_cache.len();
            let texture_cap = self.cache_manager.texture_cache.cap().get();
            // Trim when cache holds more than ~1.5× what the current view
            // needs.  After navigation, cap is reset to the minimum and grows
            // via retune; during normal scrolling it overshoots by ~1.5× for
            // scroll-ahead.  Trimming back to 1.25× releases excess without
            // causing visible flashing.
            let excess_threshold =
                (texture_keep + (texture_keep / 2)).max(MIN_DYNAMIC_TEXTURE_CACHE_ITEMS);
            if texture_count > excess_threshold || texture_cap > excess_threshold {
                let target = texture_keep
                    .saturating_add(texture_keep / 4)
                    .max(MIN_DYNAMIC_TEXTURE_CACHE_ITEMS);
                let mut visible_for_proactive = self.visible_grid_paths_snapshot();
                if let Some(detail_panel_paths) = self.detail_panel_folder_preview_paths_for_trim()
                {
                    visible_for_proactive
                        .get_or_insert_with(FxHashSet::default)
                        .extend(detail_panel_paths);
                }
                self.cache_manager.trim_thumbnail_caches(
                    target,
                    self.current_thumbnail_rgba_budget_bytes(),
                    self.current_dynamic_folder_preview_keep_count(),
                    visible_for_proactive.as_ref(),
                );
                log::debug!(
                    "[MEMORY] proactive trim: textures={}/{} target={} visible_keep={}",
                    texture_count,
                    texture_cap,
                    target,
                    texture_keep,
                );
            }
        }

        let pressure = classify_memory_pressure(process_memory);
        if pressure == MemoryPressure::None {
            return;
        }

        let aggressive = pressure == MemoryPressure::Hard;
        let is_burst = self.is_in_restore_burst();
        let pending_thumbnail_bytes = self.pending_thumbnail_rgba_bytes();
        let visible_paths_for_pending = self.visible_grid_paths_snapshot();
        self.trim_pending_thumbnail_uploads_to_limit(
            pending_thumbnail_bytes,
            visible_paths_for_pending.as_ref(),
        );
        let visible_grid_items = self.visible_grid_items_for_cache();
        let mut visible_paths = self.visible_grid_paths_snapshot();
        if let Some(detail_panel_paths) = self.detail_panel_folder_preview_paths_for_trim() {
            visible_paths
                .get_or_insert_with(FxHashSet::default)
                .extend(detail_panel_paths);
        }
        let texture_keep = self.current_dynamic_texture_keep_count();
        let folder_preview_keep = self
            .current_dynamic_folder_preview_keep_count()
            .max(self.idle_folder_preview_keep_count());
        let rgba_budget = self.current_thumbnail_rgba_budget_bytes();
        let max_pending = self.current_pending_thumbnail_upload_limit();

        let (textures_removed, rgba_removed, folder_previews_removed) = if is_burst && !aggressive {
            // Skip texture/RGBA trimming during burst — we need the caches full.
            (0, 0, 0)
        } else if aggressive {
            let texture_keep = if self.uses_aggressive_gpu_memory_policy() {
                texture_keep
            } else {
                texture_keep.max(96)
            };
            let folder_preview_keep = if self.uses_aggressive_gpu_memory_policy() {
                folder_preview_keep
            } else {
                folder_preview_keep.max(72)
            };
            self.cache_manager.trim_thumbnail_caches(
                texture_keep,
                if self.uses_aggressive_gpu_memory_policy() {
                    MIN_RGBA_BUDGET_BYTES
                } else {
                    dynamic_rgba_budget_bytes(
                        visible_grid_items,
                        self.current_thumbnail_bucket_size(),
                        MIN_RGBA_BUDGET_BYTES,
                    )
                },
                folder_preview_keep,
                visible_paths.as_ref(),
            )
        } else {
            self.cache_manager.trim_thumbnail_caches(
                texture_keep,
                rgba_budget,
                folder_preview_keep,
                visible_paths.as_ref(),
            )
        };

        if aggressive {
            self.directory_cache.clear();
            self.visible_paths_cache.clear();
            self.visible_range_cached = None;
        }
        // attempted_thumbnail_bucket is an LRU and remains bounded. Preserve it
        // under pressure so maximum-quality failures do not restart upload churn.

        // Reuse existing GIF cleanup policy (TTL + bounded memory) without forcing visible preview drop.
        self.gif_manager.cleanup(false);

        // Trim per-path icon and extension caches.  These LRU caches hold GPU
        // texture handles (each ~16–256 KB RGBA) and are never trimmed by the
        // thumbnail pipeline.  Under memory pressure we cap them at half their
        // maximum capacity; in soft mode we keep the full capacity.
        let (icon_cap, ext_cap) = if aggressive { (128, 128) } else { (256, 256) };
        let (icon_evicted, ext_evicted) = self.item_icon_loader.trim_icon_caches(icon_cap, ext_cap);

        if textures_removed > 0
            || rgba_removed > 0
            || folder_previews_removed > 0
            || icon_evicted > 0
            || ext_evicted > 0
        {
            log::debug!(
                "[MEMORY] ws={:.1}MB private={:.1}MB -> trimmed textures={} rgba={} folder_previews={} pending={} icons={} ext_icons={} mode={}",
                working_set_bytes as f64 / 1024.0 / 1024.0,
                process_memory.private_usage_bytes as f64 / 1024.0 / 1024.0,
                textures_removed,
                rgba_removed,
                folder_previews_removed,
                max_pending,
                icon_evicted,
                ext_evicted,
                if aggressive { "hard" } else { "soft" }
            );
        }
    }

    fn run_gpu_idle_working_set_trim(
        &mut self,
        working_set_bytes: u64,
        working_set_trim_blocked: bool,
    ) {
        if !self.uses_aggressive_gpu_memory_policy()
            || working_set_trim_blocked
            || self.last_user_activity.elapsed() < LOW_RAM_GPU_IDLE_WS_TRIM_AFTER
            || self.last_restore_time.elapsed() < WORKING_SET_TRIM_MIN_INTERVAL
            || working_set_bytes < LOW_RAM_GPU_IDLE_WS_TRIM_MIN_BYTES
        {
            return;
        }

        request_process_working_set_trim_series(
            format!(
                "gpu idle backend={} ws={:.1}MB",
                self.active_gpu_backend,
                working_set_bytes as f64 / 1024.0 / 1024.0
            ),
            WORKING_SET_TRIM_FOLLOW_UP_DELAYS,
            false,
        );
    }

    pub(crate) fn estimated_visible_grid_items(&self) -> usize {
        if !matches!(self.view_mode, ViewMode::Grid)
            || self.navigation_state.is_computer_view
            || self.navigation_state.is_recycle_bin_view
        {
            return 0;
        }

        let screen = self.ui_ctx.viewport_rect();
        let mut central_width = screen.width()
            - if self.show_left_sidebar {
                self.layout.sidebar_left_width.clamp(150.0, 500.0)
            } else {
                0.0
            }
            - if self.show_preview_panel {
                self.layout.sidebar_right_width.clamp(250.0, 500.0)
            } else {
                0.0
            };
        central_width = (central_width - 24.0).max(0.0);

        let thumbnail_size = self.thumbnail_size.max(crate::ui::theme::THUMBNAIL_MIN);
        let padding = 8.0;
        let cols = ((central_width - padding) / (thumbnail_size + padding))
            .floor()
            .max(1.0) as usize;

        let central_height = (screen.height() - 72.0).max(0.0);
        let row_height = thumbnail_size + 20.0 + padding;
        let rows = (central_height / row_height).ceil().max(1.0) as usize;

        cols.saturating_mul(rows.saturating_add(2))
            .clamp(0, MAX_DYNAMIC_TEXTURE_CACHE_ITEMS)
    }

    /// Total number of folder-like entries in the directories currently being
    /// rendered. Used to size the folder preview cache so it never thrashes
    /// when every folder slot is asking for its preview each frame.
    pub(crate) fn current_directory_folder_count(&self) -> usize {
        let mut count = self
            .items
            .iter()
            .filter(|item| item.is_dir && !item.is_archive())
            .count();

        if self.dual_panel_enabled {
            if let Some(snapshot) = self.dual_panel_inactive_state.as_ref() {
                count = count.saturating_add(
                    visible_items_for_snapshot(snapshot)
                        .iter()
                        .filter(|item| item.is_dir && !item.is_archive())
                        .count(),
                );
            }
        }

        count.min(MAX_DYNAMIC_FOLDER_PREVIEW_ITEMS)
    }

    pub(crate) fn thumbnail_caches_active(&self) -> bool {
        if panel_thumbnail_caches_active(
            self.view_mode,
            self.navigation_state.is_computer_view,
            self.navigation_state.is_recycle_bin_view,
            self.items.len(),
        ) {
            return true;
        }

        if detail_panel_thumbnail_active(
            self.show_preview_panel,
            self.multi_selection.len(),
            self.selected_file.as_ref(),
        ) {
            return true;
        }

        self.dual_panel_enabled
            && self
                .dual_panel_inactive_state
                .as_ref()
                .is_some_and(|snapshot| {
                    panel_thumbnail_caches_active(
                        snapshot.view_mode,
                        snapshot.is_computer_view,
                        snapshot.is_recycle_bin_view,
                        visible_items_for_snapshot(snapshot).len(),
                    )
                })
    }

    pub(crate) fn visible_grid_items_for_cache(&self) -> usize {
        let mut visible_items = 0usize;

        if panel_thumbnail_caches_active(
            self.view_mode,
            self.navigation_state.is_computer_view,
            self.navigation_state.is_recycle_bin_view,
            self.items.len(),
        ) {
            visible_items = visible_items.saturating_add(
                if crate::application::grouping::is_grouping_rendered(
                    self.view_mode,
                    &self.group_projection,
                ) {
                    self.visible_group_paths.len()
                } else {
                    visible_count_from_range(self.items.len(), self.visible_index_range)
                        .unwrap_or_else(|| self.estimated_visible_grid_items())
                },
            );
        }

        if self.dual_panel_enabled {
            if let Some(snapshot) = self.dual_panel_inactive_state.as_ref() {
                let inactive_items = visible_items_for_snapshot(snapshot);
                if panel_thumbnail_caches_active(
                    snapshot.view_mode,
                    snapshot.is_computer_view,
                    snapshot.is_recycle_bin_view,
                    inactive_items.len(),
                ) {
                    visible_items = visible_items.saturating_add(
                        if crate::application::grouping::is_grouping_rendered(
                            snapshot.view_mode,
                            &snapshot.group_projection,
                        ) {
                            snapshot.visible_group_paths.len()
                        } else {
                            visible_count_from_range(
                                inactive_items.len(),
                                snapshot.visible_index_range,
                            )
                            .unwrap_or_else(|| self.estimated_visible_grid_items())
                        },
                    );
                }
            }
        }

        if visible_items == 0 {
            self.estimated_visible_grid_items()
        } else {
            visible_items.clamp(0, MAX_DYNAMIC_TEXTURE_CACHE_ITEMS)
        }
    }

    pub(crate) fn visible_grid_paths_snapshot(&mut self) -> Option<FxHashSet<std::path::PathBuf>> {
        self.visible_paths_cache.clear();
        self.visible_range_cached = self.visible_index_range;

        if matches!(
            self.view_mode,
            ViewMode::Grid | ViewMode::List | ViewMode::ColumnList | ViewMode::Miller
        ) {
            if crate::application::grouping::is_grouping_rendered(
                self.view_mode,
                &self.group_projection,
            ) {
                self.visible_paths_cache
                    .extend(self.visible_group_paths.iter().cloned());
            } else {
                insert_visible_paths_from_range(
                    &mut self.visible_paths_cache,
                    self.items.as_ref().as_slice(),
                    self.visible_index_range,
                );
            }
        }

        if self.dual_panel_enabled {
            if let Some(snapshot) = self.dual_panel_inactive_state.as_ref() {
                if matches!(
                    snapshot.view_mode,
                    ViewMode::Grid | ViewMode::List | ViewMode::ColumnList | ViewMode::Miller
                ) {
                    if crate::application::grouping::is_grouping_rendered(
                        snapshot.view_mode,
                        &snapshot.group_projection,
                    ) {
                        self.visible_paths_cache
                            .extend(snapshot.visible_group_paths.iter().cloned());
                    } else {
                        insert_visible_paths_from_range(
                            &mut self.visible_paths_cache,
                            visible_items_for_snapshot(snapshot),
                            snapshot.visible_index_range,
                        );
                    }
                }
            }
        }

        if self.show_preview_panel && self.multi_selection.len() <= 1 {
            if let Some(selected) = self.selected_file.as_ref() {
                self.visible_paths_cache.insert(selected.path.clone());
            } else if !self.navigation_state.is_computer_view
                && !self.navigation_state.is_recycle_bin_view
            {
                self.visible_paths_cache.insert(std::path::PathBuf::from(
                    &self.navigation_state.current_path,
                ));
            }
        }

        if self.visible_paths_cache.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.visible_paths_cache))
        }
    }
}

pub(crate) fn dynamic_texture_keep_count(visible_grid_items: usize) -> usize {
    let target = visible_grid_items.saturating_mul(3).saturating_add(1) / 2;

    target.clamp(
        MIN_DYNAMIC_TEXTURE_CACHE_ITEMS,
        MAX_DYNAMIC_TEXTURE_CACHE_ITEMS,
    )
}

pub(crate) fn dynamic_folder_preview_keep_count(
    visible_grid_items: usize,
    directory_folder_items: usize,
) -> usize {
    let viewport_target = visible_grid_items.saturating_mul(3).saturating_add(1) / 2;

    // Anti-thrash floor: when the renderer can request a preview for any folder
    // currently displayed in the directory, the cache must fit at least all of
    // them. Otherwise every upload evicts a path that is re-requested in the
    // following frame, producing a constant `ctx.load_texture` storm and a
    // steady GPU staging-buffer leak.
    viewport_target.max(directory_folder_items).clamp(
        MIN_DYNAMIC_FOLDER_PREVIEW_ITEMS,
        MAX_DYNAMIC_FOLDER_PREVIEW_ITEMS,
    )
}

pub(crate) fn dynamic_rgba_budget_bytes(
    visible_grid_items: usize,
    bucket_size: u32,
    floor_bytes: usize,
) -> usize {
    let bucket_bytes = (bucket_size as usize)
        .saturating_mul(bucket_size as usize)
        .saturating_mul(4);
    let target = visible_grid_items
        .saturating_mul(bucket_bytes)
        .saturating_mul(3)
        .saturating_add(3)
        / 4;

    target.clamp(floor_bytes, MAX_RGBA_BUDGET_BYTES)
}

fn request_process_working_set_trim_series(
    reason: String,
    delays: &'static [Duration],
    require_minimized: bool,
) -> bool {
    if process_working_set_trim_disabled() {
        return false;
    }
    if delays.is_empty() {
        return false;
    }

    let now = Instant::now();
    let mut last_trim = LAST_WORKING_SET_TRIM_REQUEST
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if last_trim
        .as_ref()
        .is_some_and(|last| now.duration_since(*last) < WORKING_SET_TRIM_MIN_INTERVAL)
    {
        return false;
    }
    *last_trim = Some(now);
    drop(last_trim);

    let trim_epoch = WORKING_SET_TRIM_EPOCH.load(Ordering::Acquire);
    let spawn_result = std::thread::Builder::new()
        .name("mtt-working-set-trim".to_string())
        .stack_size(128 * 1024)
        .spawn(move || {
            let mut elapsed = Duration::ZERO;
            let mut trimmed_any = false;
            for delay in delays {
                if *delay > elapsed {
                    std::thread::sleep(*delay - elapsed);
                    elapsed = *delay;
                }
                if working_set_trim_cancelled(
                    WORKING_SET_TRIM_BLOCKED.load(Ordering::Acquire),
                    WORKING_SET_TRIM_EPOCH.load(Ordering::Acquire),
                    trim_epoch,
                ) {
                    log::debug!("[MEMORY] working-set trim cancelled because the app became busy");
                    if !trimmed_any {
                        clear_working_set_trim_request(now);
                    }
                    break;
                }
                if trim_process_working_set(&reason, trim_epoch, require_minimized) {
                    trimmed_any = true;
                } else if working_set_trim_cancelled(
                    WORKING_SET_TRIM_BLOCKED.load(Ordering::Acquire),
                    WORKING_SET_TRIM_EPOCH.load(Ordering::Acquire),
                    trim_epoch,
                ) {
                    if !trimmed_any {
                        clear_working_set_trim_request(now);
                    }
                    break;
                }
            }
        });

    match spawn_result {
        Ok(_) => true,
        Err(error) => {
            clear_working_set_trim_request(now);
            log::debug!("[MEMORY] failed to spawn working-set trim: {error}");
            false
        }
    }
}

fn clear_working_set_trim_request(requested_at: Instant) {
    let mut last_trim = LAST_WORKING_SET_TRIM_REQUEST
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if *last_trim == Some(requested_at) {
        *last_trim = None;
    }
}

fn process_working_set_trim_disabled() -> bool {
    std::env::var("MTT_DISABLE_WS_TRIM")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn trim_process_working_set(reason: &str, scheduled_epoch: u64, require_minimized: bool) -> bool {
    let _execution_guard = working_set_trim_execution_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if working_set_trim_cancelled(
        WORKING_SET_TRIM_BLOCKED.load(Ordering::Acquire),
        WORKING_SET_TRIM_EPOCH.load(Ordering::Acquire),
        scheduled_epoch,
    ) {
        return false;
    }
    if require_minimized
        && crate::infrastructure::windows::window_subclass::layout_phase()
            != crate::infrastructure::windows::window_subclass::WindowLayoutPhase::Minimized
    {
        return false;
    }

    unsafe {
        use windows::Win32::System::Memory::{
            SetProcessWorkingSetSizeEx, SETPROCESSWORKINGSETSIZEEX_FLAGS,
        };
        use windows::Win32::System::Threading::GetCurrentProcess;

        let before = current_process_memory_snapshot();
        let process = GetCurrentProcess();
        match SetProcessWorkingSetSizeEx(
            process,
            usize::MAX,
            usize::MAX,
            SETPROCESSWORKINGSETSIZEEX_FLAGS(0),
        ) {
            Ok(()) => {
                let after = current_process_memory_snapshot();
                if let (Some(before), Some(after)) = (before, after) {
                    let effective = working_set_trim_was_effective(before, after);
                    if effective {
                        LAST_EFFECTIVE_WORKING_SET_TRIM_BYTES
                            .store(after.working_set_bytes, Ordering::Release);
                        WORKING_SET_TRIM_SUCCESS_COUNT.fetch_add(1, Ordering::AcqRel);
                    }
                    log::debug!(
                        "[MEMORY] working-set trim API succeeded after {}: effective={} ws={:.1}->{:.1}MB private={:.1}->{:.1}MB",
                        reason,
                        effective,
                        bytes_to_mb(before.working_set_bytes),
                        bytes_to_mb(after.working_set_bytes),
                        bytes_to_mb(before.private_usage_bytes),
                        bytes_to_mb(after.private_usage_bytes),
                    );
                    crate::infrastructure::diagnostic_logger::diag_info(
                        "memory_trim",
                        "api_succeeded",
                        &[
                            crate::infrastructure::diagnostic_logger::field_u64(
                                "working_set_before_bytes",
                                before.working_set_bytes,
                            ),
                            crate::infrastructure::diagnostic_logger::field_u64(
                                "working_set_after_bytes",
                                after.working_set_bytes,
                            ),
                            crate::infrastructure::diagnostic_logger::field_u64(
                                "private_before_bytes",
                                before.private_usage_bytes,
                            ),
                            crate::infrastructure::diagnostic_logger::field_u64(
                                "private_after_bytes",
                                after.private_usage_bytes,
                            ),
                            crate::infrastructure::diagnostic_logger::field_bool(
                                "effective",
                                effective,
                            ),
                        ],
                    );
                    effective
                } else {
                    log::debug!("[MEMORY] working-set trim API succeeded after {reason}");
                    crate::infrastructure::diagnostic_logger::diag_info(
                        "memory_trim",
                        "api_succeeded_without_snapshot",
                        &[],
                    );
                    false
                }
            }
            Err(error) => {
                log::debug!("[MEMORY] working-set trim failed after {reason}: {error}");
                crate::infrastructure::diagnostic_logger::diag_warn("memory_trim", "failed", &[]);
                false
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn trim_process_working_set(
    _reason: &str,
    _scheduled_epoch: u64,
    _require_minimized: bool,
) -> bool {
    false
}

#[cfg(target_os = "windows")]
fn current_process_memory_snapshot() -> Option<ProcessMemorySnapshot> {
    use windows::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;

    unsafe {
        let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
        if K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        )
        .as_bool()
        {
            Some(ProcessMemorySnapshot {
                working_set_bytes: counters.WorkingSetSize as u64,
                private_usage_bytes: counters.PrivateUsage as u64,
            })
        } else {
            None
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn current_process_memory_snapshot() -> Option<ProcessMemorySnapshot> {
    None
}

#[cfg(test)]
mod inactive_panel_paths_tests {
    use super::{
        backend_uses_conservative_thumbnail_upload_policy, backend_uses_low_ram_gpu_policy,
        background_trim_should_rearm, classify_memory_pressure, detail_panel_thumbnail_active,
        insert_item_reference_paths, pending_thumbnail_eviction_index,
        trim_pending_thumbnail_queue, working_set_trim_cancelled, working_set_trim_was_effective,
        FileEntry, FxHashSet, MemoryPressure, ProcessMemorySnapshot,
        BACKGROUND_WS_TRIM_REARM_GROWTH_BYTES, LOW_RAM_GPU_IDLE_WS_TRIM_MIN_BYTES,
        WORKING_SET_TRIM_EFFECTIVE_REDUCTION_BYTES,
    };
    use crate::domain::file_entry::SyncStatus;
    use crate::domain::thumbnail::ThumbnailData;
    use crate::infrastructure::io_priority::IOPriority;
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn entry(path: &str, cover: Option<&str>) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            name: String::new(),
            is_dir: false,
            size: 0,
            modified: 0,
            created: None,
            folder_cover: cover.map(PathBuf::from),
            drive_info: None,
            sync_status: SyncStatus::None,
            is_hidden: false,
            recycle_bin: None,
        }
    }

    /// The lazily-built set must answer membership identically to the old
    /// per-item linear scan: item path, folder cover, and selection are
    /// recognized; unrelated (stale external) paths are rejected.
    #[test]
    fn set_membership_matches_reference_scan() {
        let items = [
            entry(r"C:\a\file1.jpg", None),
            entry(r"C:\a\folder", Some(r"C:\a\folder\cover.png")),
        ];
        let selected = entry(r"C:\a\sel.mp4", Some(r"C:\a\sel_cover.png"));

        let mut set = FxHashSet::default();
        insert_item_reference_paths(&mut set, &selected);
        for item in &items {
            insert_item_reference_paths(&mut set, item);
        }

        // Reference scan equivalent to the removed linear implementation.
        let reference = |path: &PathBuf| -> bool {
            let refs = |item: &FileEntry| {
                &item.path == path || item.folder_cover.as_ref().is_some_and(|c| c == path)
            };
            refs(&selected) || items.iter().any(refs)
        };

        for probe in [
            r"C:\a\file1.jpg",
            r"C:\a\folder",
            r"C:\a\folder\cover.png",
            r"C:\a\sel.mp4",
            r"C:\a\sel_cover.png",
            r"C:\a\missing.txt",
        ] {
            let p = PathBuf::from(probe);
            assert_eq!(set.contains(&p), reference(&p), "mismatch for {probe}");
        }
    }

    #[test]
    fn selected_media_keeps_thumbnail_cache_active_for_empty_view() {
        let video = entry(r"C:\a\video.mp4", None);
        let document = entry(r"C:\a\document.txt", None);

        assert!(detail_panel_thumbnail_active(true, 1, Some(&video)));
        assert!(!detail_panel_thumbnail_active(false, 1, Some(&video)));
        assert!(!detail_panel_thumbnail_active(true, 2, Some(&video)));
        assert!(!detail_panel_thumbnail_active(true, 1, Some(&document)));
        assert!(!detail_panel_thumbnail_active(true, 0, None));
    }

    #[test]
    fn low_ram_gpu_policy_includes_all_selectable_backends() {
        assert!(backend_uses_low_ram_gpu_policy("glow"));
        assert!(backend_uses_low_ram_gpu_policy("Vulkan"));
        assert!(backend_uses_low_ram_gpu_policy("Dx12"));
        assert!(!backend_uses_low_ram_gpu_policy(""));
    }

    #[test]
    fn conservative_thumbnail_upload_policy_includes_wgpu_backends() {
        assert!(backend_uses_conservative_thumbnail_upload_policy("Vulkan"));
        assert!(backend_uses_conservative_thumbnail_upload_policy("Dx12"));
        assert!(!backend_uses_conservative_thumbnail_upload_policy("glow"));
        assert!(!backend_uses_conservative_thumbnail_upload_policy(""));
    }

    #[test]
    fn private_commit_triggers_memory_pressure_cleanup() {
        let mb = 1024 * 1024;
        assert_eq!(
            classify_memory_pressure(ProcessMemorySnapshot {
                working_set_bytes: 100 * mb,
                private_usage_bytes: 600 * mb,
            }),
            MemoryPressure::Soft
        );
        assert_eq!(
            classify_memory_pressure(ProcessMemorySnapshot {
                working_set_bytes: 100 * mb,
                private_usage_bytes: 750 * mb,
            }),
            MemoryPressure::Hard
        );
        assert_eq!(
            classify_memory_pressure(ProcessMemorySnapshot {
                working_set_bytes: 500 * mb,
                private_usage_bytes: 500 * mb,
            }),
            MemoryPressure::None
        );
    }

    #[test]
    fn pending_thumbnail_eviction_preserves_selected_then_visible() {
        let thumbnail = |path: &str| ThumbnailData {
            path: PathBuf::from(path),
            image_data: Arc::new(vec![0; 4]),
            width: 1,
            height: 1,
            generation: 0,
            request_epoch: 0,
            priority: IOPriority::Interactive,
            not_found: false,
            premultiplied: true,
        };
        let pending = VecDeque::from([
            thumbnail("selected"),
            thumbnail("visible"),
            thumbnail("offscreen"),
        ]);
        let selected = PathBuf::from("selected");
        let visible = FxHashSet::from_iter([selected.clone(), PathBuf::from("visible")]);

        assert_eq!(
            pending_thumbnail_eviction_index(&pending, Some(&visible), Some(&selected)),
            Some(2)
        );

        let newest_visible = PathBuf::from("newest-visible");
        let pending = VecDeque::from([
            thumbnail("selected"),
            thumbnail("visible"),
            thumbnail("newest-visible"),
        ]);
        let visible =
            FxHashSet::from_iter([selected.clone(), PathBuf::from("visible"), newest_visible]);
        assert_eq!(
            pending_thumbnail_eviction_index(&pending, Some(&visible), Some(&selected)),
            None
        );

        let pending = VecDeque::from([thumbnail("selected")]);
        assert_eq!(
            pending_thumbnail_eviction_index(&pending, Some(&visible), Some(&selected)),
            None
        );
    }

    #[test]
    fn pending_thumbnail_trim_enforces_count_and_byte_limits() {
        let thumbnail = |path: &str, bytes: usize| ThumbnailData {
            path: PathBuf::from(path),
            image_data: Arc::new(vec![0; bytes]),
            width: 1,
            height: 1,
            generation: 0,
            request_epoch: 0,
            priority: IOPriority::Interactive,
            not_found: false,
            premultiplied: true,
        };
        let selected = PathBuf::from("selected");
        let visible = FxHashSet::from_iter([selected.clone(), PathBuf::from("visible")]);
        let mut pending = VecDeque::from([
            thumbnail("selected", 4),
            thumbnail("visible", 4),
            thumbnail("offscreen", 4),
        ]);

        let (removed, final_bytes) =
            trim_pending_thumbnail_queue(&mut pending, 12, 2, 8, Some(&visible), Some(&selected));
        assert_eq!(
            removed
                .iter()
                .map(|thumbnail| thumbnail.path.as_path())
                .collect::<Vec<_>>(),
            vec![std::path::Path::new("offscreen")]
        );
        assert_eq!(pending.len(), 2);
        assert_eq!(final_bytes, 8);
        assert_eq!(
            pending
                .iter()
                .map(|thumbnail| thumbnail.image_data.len())
                .sum::<usize>(),
            8
        );

        let (removed, final_bytes) =
            trim_pending_thumbnail_queue(&mut pending, 8, 2, 4, Some(&visible), Some(&selected));
        assert!(removed.is_empty());
        assert_eq!(pending.len(), 2);
        assert_eq!(final_bytes, 8);
    }

    #[test]
    fn working_set_trim_is_cancelled_by_activity_or_epoch_change() {
        assert!(!working_set_trim_cancelled(false, 7, 7));
        assert!(working_set_trim_cancelled(true, 7, 7));
        assert!(working_set_trim_cancelled(false, 8, 7));
    }

    #[test]
    fn working_set_trim_requires_an_observable_reduction() {
        let before = ProcessMemorySnapshot {
            working_set_bytes: 128 * 1024 * 1024,
            private_usage_bytes: 160 * 1024 * 1024,
        };
        assert!(!working_set_trim_was_effective(
            before,
            ProcessMemorySnapshot {
                working_set_bytes: before.working_set_bytes
                    - WORKING_SET_TRIM_EFFECTIVE_REDUCTION_BYTES
                    + 1,
                private_usage_bytes: before.private_usage_bytes,
            }
        ));
        assert!(working_set_trim_was_effective(
            before,
            ProcessMemorySnapshot {
                working_set_bytes: before.working_set_bytes
                    - WORKING_SET_TRIM_EFFECTIVE_REDUCTION_BYTES,
                private_usage_bytes: before.private_usage_bytes,
            }
        ));
    }

    #[test]
    fn minimized_monitor_rearms_only_after_memory_grows_past_the_limit() {
        assert!(!background_trim_should_rearm(
            false,
            false,
            LOW_RAM_GPU_IDLE_WS_TRIM_MIN_BYTES,
            0,
        ));
        assert!(!background_trim_should_rearm(
            true,
            true,
            LOW_RAM_GPU_IDLE_WS_TRIM_MIN_BYTES,
            0,
        ));
        assert!(!background_trim_should_rearm(
            true,
            false,
            LOW_RAM_GPU_IDLE_WS_TRIM_MIN_BYTES - 1,
            0,
        ));
        assert!(!background_trim_should_rearm(
            true,
            false,
            LOW_RAM_GPU_IDLE_WS_TRIM_MIN_BYTES + BACKGROUND_WS_TRIM_REARM_GROWTH_BYTES,
            LOW_RAM_GPU_IDLE_WS_TRIM_MIN_BYTES + BACKGROUND_WS_TRIM_REARM_GROWTH_BYTES - 1,
        ));
        assert!(background_trim_should_rearm(
            true,
            false,
            LOW_RAM_GPU_IDLE_WS_TRIM_MIN_BYTES + BACKGROUND_WS_TRIM_REARM_GROWTH_BYTES,
            LOW_RAM_GPU_IDLE_WS_TRIM_MIN_BYTES,
        ));
    }
}
