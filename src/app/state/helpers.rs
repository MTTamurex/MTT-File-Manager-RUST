use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::domain::file_entry::FileEntry;
use crate::domain::file_entry::ViewMode;
use crate::ui::cache::{
    FxHashSet, DEFAULT_DYNAMIC_RGBA_BUDGET_BYTES, MAX_DYNAMIC_FOLDER_PREVIEW_ITEMS,
    MAX_DYNAMIC_TEXTURE_CACHE_ITEMS, MAX_RGBA_BUDGET_BYTES, MIN_DYNAMIC_FOLDER_PREVIEW_ITEMS,
    MIN_DYNAMIC_TEXTURE_CACHE_ITEMS, MIN_RGBA_BUDGET_BYTES,
    VULKAN_MAX_DYNAMIC_FOLDER_PREVIEW_ITEMS, VULKAN_MAX_DYNAMIC_TEXTURE_CACHE_ITEMS,
};
use crate::workers::thumbnail::processing::get_bucket_size;

use super::ImageViewerApp;

const BASE_PENDING_THUMBNAILS: usize = 64;
const MIN_DYNAMIC_PENDING_THUMBNAILS: usize = 16;
const MAX_DYNAMIC_PENDING_THUMBNAILS: usize = 1024;
const MAX_PENDING_THUMBNAIL_RGBA_BYTES: usize = 64 * 1024 * 1024;
const VULKAN_MAX_PENDING_THUMBNAIL_RGBA_BYTES: usize = 16 * 1024 * 1024;
const VULKAN_RGBA_BUDGET_FLOOR_BYTES: usize = MIN_RGBA_BUDGET_BYTES;
const VULKAN_MAX_RGBA_BUDGET_BYTES: usize = 8 * 1024 * 1024;
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
const VULKAN_IDLE_WS_TRIM_AFTER: Duration = Duration::from_secs(8);
const VULKAN_IDLE_WS_TRIM_MIN_BYTES: u64 = 24 * 1024 * 1024;

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

fn panel_thumbnail_caches_active(
    view_mode: ViewMode,
    is_computer_view: bool,
    is_recycle_bin_view: bool,
    item_count: usize,
) -> bool {
    matches!(view_mode, ViewMode::Grid | ViewMode::List)
        && !is_computer_view
        && !is_recycle_bin_view
        && item_count > 0
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

fn item_references_path(item: &FileEntry, path: &std::path::PathBuf) -> bool {
    &item.path == path
        || item
            .folder_cover
            .as_ref()
            .is_some_and(|cover| cover == path)
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
    for idx in min_idx..=max_idx {
        visible_paths.insert(items[idx].path.clone());
    }
}

impl ImageViewerApp {
    pub(crate) fn all_items_mut(&mut self) -> &mut Vec<FileEntry> {
        Arc::make_mut(&mut self.all_items)
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
        self.clear_pending_items_rebuild_flags();
        self.last_items_rebuild = Instant::now();
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

    pub(crate) fn path_belongs_to_inactive_panel(&self, path: &std::path::PathBuf) -> bool {
        self.dual_panel_enabled
            && self
                .dual_panel_inactive_state
                .as_ref()
                .is_some_and(|snapshot| {
                    snapshot
                        .selected_file
                        .as_ref()
                        .is_some_and(|selected| item_references_path(selected, path))
                        || visible_items_for_snapshot(snapshot)
                            .iter()
                            .any(|item| item_references_path(item, path))
                })
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
    /// backends (Glow and Wgpu-GL).
    pub fn is_opengl_backend(&self) -> bool {
        matches!(self.active_gpu_backend.as_str(), "Gl" | "glow")
    }

    /// Returns `true` when the active wgpu backend is Vulkan.
    /// Vulkan has the best throughput in this app, but queued texture uploads can
    /// hold staging/RGBA memory longer than the generic wgpu path expects.
    pub fn is_vulkan_backend(&self) -> bool {
        self.active_gpu_backend == "Vulkan"
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

    pub fn is_video_docked_visible(&self) -> bool {
        if let Some(preview) = &self.media_preview {
            !preview.is_detached() && preview.is_visible()
        } else {
            false
        }
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
        if self.is_vulkan_backend() {
            let cap = VULKAN_MAX_DYNAMIC_TEXTURE_CACHE_ITEMS
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
        let target = if self.is_vulkan_backend() {
            // Vulkan reloads previews quickly; prefer releasing offscreen folders
            // over holding every folder in large directories.
            dynamic_texture_keep_count(visible_items)
        } else {
            dynamic_folder_preview_keep_count(visible_items, self.current_directory_folder_count())
        };
        if self.is_vulkan_backend() {
            let cap = VULKAN_MAX_DYNAMIC_FOLDER_PREVIEW_ITEMS
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
        let floor_bytes = if self.is_vulkan_backend() {
            VULKAN_RGBA_BUDGET_FLOOR_BYTES
        } else {
            DEFAULT_DYNAMIC_RGBA_BUDGET_BYTES
        };
        let budget = self.current_dynamic_rgba_budget_bytes(floor_bytes);

        if self.is_vulkan_backend() && self.thumbnail_caches_active() {
            budget
                .min(VULKAN_MAX_RGBA_BUDGET_BYTES)
                .max(MIN_RGBA_BUDGET_BYTES)
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

        if self.is_vulkan_backend() {
            VULKAN_MAX_PENDING_THUMBNAIL_RGBA_BYTES
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
            .max(BASE_PENDING_THUMBNAILS)
            .min(MAX_DYNAMIC_PENDING_THUMBNAILS)
            .min(byte_limited_items)
    }

    fn pending_thumbnail_rgba_bytes(&self) -> usize {
        self.pending_thumbnails
            .iter()
            .map(|thumbnail| thumbnail.image_data.len())
            .sum()
    }

    pub(crate) fn trim_pending_thumbnail_uploads_to_limit(&mut self) {
        let max_pending = self.current_pending_thumbnail_upload_limit();
        let max_pending_bytes = self.current_pending_thumbnail_upload_byte_limit();
        let mut pending_bytes = self.pending_thumbnail_rgba_bytes();
        if self.pending_thumbnails.len() <= max_pending && pending_bytes <= max_pending_bytes {
            return;
        }

        let visible_paths = self.visible_grid_paths_snapshot();
        let selected_path = self.selected_file.as_ref().map(|file| file.path.clone());
        while self.pending_thumbnails.len() > max_pending || pending_bytes > max_pending_bytes {
            let evict_idx = self.pending_thumbnails.iter().position(|thumb| {
                let is_selected = selected_path.as_ref() == Some(&thumb.path);
                let is_visible = visible_paths
                    .as_ref()
                    .is_some_and(|visible_paths| visible_paths.contains(&thumb.path));

                !is_selected && !is_visible
            });

            let old = if let Some(evict_idx) = evict_idx {
                self.pending_thumbnails.remove(evict_idx)
            } else if visible_paths.is_none() {
                self.pending_thumbnails
                    .iter()
                    .position(|thumb| selected_path.as_ref() != Some(&thumb.path))
                    .and_then(|idx| self.pending_thumbnails.remove(idx))
            } else {
                None
            };

            if let Some(old) = old {
                pending_bytes = pending_bytes.saturating_sub(old.image_data.len());
                self.cache_manager.finish_pending_upload(&old.path);
            } else {
                break;
            }
        }
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
        self.cache_manager
            .attempted_thumbnail_bucket
            .shrink_to_fit();
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
            if self.is_vulkan_backend() {
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

            if self.is_vulkan_backend() {
                request_process_working_set_trim_series(
                    format!("vulkan thumbnail navigation ({reason})"),
                    WORKING_SET_TRIM_FOLLOW_UP_DELAYS,
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

        if matches!(snapshot.view_mode, ViewMode::Grid | ViewMode::List) {
            insert_visible_paths_from_range(
                &mut visible_paths,
                visible_items_for_snapshot(snapshot),
                snapshot.visible_index_range,
            );
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
            );
        }
    }

    fn run_memory_maintenance_impl(&mut self, force: bool) {
        if !force && self.last_memory_maintenance.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_memory_maintenance = Instant::now();

        let thumbnails_active = self.thumbnail_caches_active();
        if !thumbnails_active && !self.is_in_restore_burst() {
            self.release_thumbnail_pipeline_for_inactive_view("inactive-maintenance", false);
        }

        let Some(process_memory) = current_process_memory_snapshot() else {
            return;
        };
        let working_set_bytes = process_memory.working_set_bytes;
        self.run_vulkan_idle_working_set_trim(working_set_bytes);

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

        const SOFT_LIMIT_BYTES: u64 = 550 * 1024 * 1024;
        const HARD_LIMIT_BYTES: u64 = 700 * 1024 * 1024;

        if working_set_bytes < SOFT_LIMIT_BYTES {
            return;
        }

        let aggressive = working_set_bytes >= HARD_LIMIT_BYTES;
        let is_burst = self.is_in_restore_burst();
        self.trim_pending_thumbnail_uploads_to_limit();
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
            let texture_keep = if self.is_vulkan_backend() {
                texture_keep
            } else {
                texture_keep.max(96)
            };
            let folder_preview_keep = if self.is_vulkan_backend() {
                folder_preview_keep
            } else {
                folder_preview_keep.max(72)
            };
            self.cache_manager.trim_thumbnail_caches(
                texture_keep,
                if self.is_vulkan_backend() {
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
            self.thumbnail_request_epochs.clear();
            self.cache_manager.attempted_thumbnail_bucket.clear();
        } else if self.cache_manager.attempted_thumbnail_bucket.len()
            > MAX_DYNAMIC_TEXTURE_CACHE_ITEMS
        {
            self.cache_manager.attempted_thumbnail_bucket.clear();
        }

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
                "[MEMORY] RAM {:.1}MB -> trimmed textures={} rgba={} folder_previews={} pending={} icons={} ext_icons={} mode={}",
                working_set_bytes as f64 / 1024.0 / 1024.0,
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

    fn run_vulkan_idle_working_set_trim(&mut self, working_set_bytes: u64) {
        if !self.is_vulkan_backend()
            || self.is_in_restore_burst()
            || self.last_user_activity.elapsed() < VULKAN_IDLE_WS_TRIM_AFTER
            || working_set_bytes < VULKAN_IDLE_WS_TRIM_MIN_BYTES
            || self.is_loading_folder
            || self.is_item_dragging
            || self.pending_drag_move_confirmation.is_some()
            || self.shell_menu_loading
        {
            return;
        }

        let thumbnail_pipeline_idle = self.thumbnail_queue.pending_count() == 0
            && self.pending_thumbnails.is_empty()
            && self.image_receiver.len() == 0
            && self.cache_manager.loading_set.is_empty()
            && self.cache_manager.folder_preview_loading.is_empty()
            && self.cache_manager.pending_upload_set.is_empty();

        if !thumbnail_pipeline_idle {
            return;
        }

        request_process_working_set_trim_series(
            format!(
                "vulkan idle ws={:.1}MB path={}",
                working_set_bytes as f64 / 1024.0 / 1024.0,
                self.navigation_state.current_path
            ),
            WORKING_SET_TRIM_FOLLOW_UP_DELAYS,
        );
    }

    pub(crate) fn estimated_visible_grid_items(&self) -> usize {
        if !matches!(self.view_mode, ViewMode::Grid)
            || self.navigation_state.is_computer_view
            || self.navigation_state.is_recycle_bin_view
        {
            return 0;
        }

        let screen = self.ui_ctx.screen_rect();
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
                visible_count_from_range(self.items.len(), self.visible_index_range)
                    .unwrap_or_else(|| self.estimated_visible_grid_items()),
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
                        visible_count_from_range(
                            inactive_items.len(),
                            snapshot.visible_index_range,
                        )
                        .unwrap_or_else(|| self.estimated_visible_grid_items()),
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

        if matches!(self.view_mode, ViewMode::Grid | ViewMode::List) {
            insert_visible_paths_from_range(
                &mut self.visible_paths_cache,
                self.items.as_ref().as_slice(),
                self.visible_index_range,
            );
        }

        if self.dual_panel_enabled {
            if let Some(snapshot) = self.dual_panel_inactive_state.as_ref() {
                if matches!(snapshot.view_mode, ViewMode::Grid | ViewMode::List) {
                    insert_visible_paths_from_range(
                        &mut self.visible_paths_cache,
                        visible_items_for_snapshot(snapshot),
                        snapshot.visible_index_range,
                    );
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

    target
        .max(MIN_DYNAMIC_TEXTURE_CACHE_ITEMS)
        .min(MAX_DYNAMIC_TEXTURE_CACHE_ITEMS)
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
    viewport_target
        .max(directory_folder_items)
        .max(MIN_DYNAMIC_FOLDER_PREVIEW_ITEMS)
        .min(MAX_DYNAMIC_FOLDER_PREVIEW_ITEMS)
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

fn request_process_working_set_trim_series(reason: String, delays: &'static [Duration]) {
    if process_working_set_trim_disabled() {
        return;
    }
    if delays.is_empty() {
        return;
    }

    static LAST_TRIM: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    let now = Instant::now();
    if let Ok(mut last_trim) = LAST_TRIM.get_or_init(|| Mutex::new(None)).lock() {
        if last_trim
            .as_ref()
            .is_some_and(|last| now.duration_since(*last) < WORKING_SET_TRIM_MIN_INTERVAL)
        {
            return;
        }
        *last_trim = Some(now);
    }

    let spawn_result = std::thread::Builder::new()
        .name("mtt-working-set-trim".to_string())
        .stack_size(128 * 1024)
        .spawn(move || {
            let mut elapsed = Duration::ZERO;
            for delay in delays {
                if *delay > elapsed {
                    std::thread::sleep(*delay - elapsed);
                    elapsed = *delay;
                }
                trim_process_working_set(&reason);
            }
        });

    if let Err(error) = spawn_result {
        log::debug!("[MEMORY] failed to spawn working-set trim: {error}");
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
fn trim_process_working_set(reason: &str) {
    unsafe {
        use windows::Win32::System::Memory::{
            SetProcessWorkingSetSizeEx, SETPROCESSWORKINGSETSIZEEX_FLAGS,
        };
        use windows::Win32::System::Threading::GetCurrentProcess;

        let process = GetCurrentProcess();
        match SetProcessWorkingSetSizeEx(
            process,
            usize::MAX,
            usize::MAX,
            SETPROCESSWORKINGSETSIZEEX_FLAGS(0),
        ) {
            Ok(()) => log::debug!("[MEMORY] trimmed working set after {reason}"),
            Err(error) => log::debug!("[MEMORY] working-set trim failed after {reason}: {error}"),
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn trim_process_working_set(_reason: &str) {}

#[cfg(target_os = "windows")]
fn current_process_memory_snapshot() -> Option<ProcessMemorySnapshot> {
    use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;

    unsafe {
        let mut counters = PROCESS_MEMORY_COUNTERS::default();
        if K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
        .as_bool()
        {
            Some(ProcessMemorySnapshot {
                working_set_bytes: counters.WorkingSetSize as u64,
                private_usage_bytes: counters.PagefileUsage as u64,
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
