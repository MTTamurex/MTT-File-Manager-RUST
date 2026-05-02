use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crate::domain::file_entry::FileEntry;
use crate::domain::file_entry::ViewMode;
use crate::ui::cache::{
    FxHashSet, DEFAULT_DYNAMIC_RGBA_BUDGET_BYTES, MAX_DYNAMIC_FOLDER_PREVIEW_ITEMS,
    MAX_DYNAMIC_TEXTURE_CACHE_ITEMS, MAX_RGBA_BUDGET_BYTES, MIN_DYNAMIC_FOLDER_PREVIEW_ITEMS,
    MIN_DYNAMIC_TEXTURE_CACHE_ITEMS, MIN_RGBA_BUDGET_BYTES,
};
use crate::workers::thumbnail::processing::get_bucket_size;

use super::ImageViewerApp;

const BASE_PENDING_THUMBNAILS: usize = 64;
const MIN_DYNAMIC_PENDING_THUMBNAILS: usize = 16;
const MAX_DYNAMIC_PENDING_THUMBNAILS: usize = 1024;
const MAX_PENDING_THUMBNAIL_RGBA_BYTES: usize = 64 * 1024 * 1024;
const MEMORY_TRACE_INTERVAL: Duration = Duration::from_secs(5);
const IDLE_THUMBNAIL_TEXTURE_KEEP: usize = 8;
const IDLE_FOLDER_PREVIEW_KEEP: usize = 0;
const IDLE_RGBA_BUDGET_BYTES: usize = 4 * 1024 * 1024;
const IDLE_PENDING_THUMBNAILS: usize = 1;

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
    matches!(view_mode, ViewMode::Grid)
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

    /// Returns `true` while the post-restore burst window is active.
    /// During burst, thumbnail upload throttling is bypassed to recover visual
    /// state quickly after the OS pages out the GPU working set.
    pub fn is_in_restore_burst(&self) -> bool {
        self.restore_burst_until
            .is_some_and(|deadline| Instant::now() < deadline)
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
        ((logical_size_px.max(1) as f32) * scale).ceil() as u32
    }

    pub(crate) fn current_thumbnail_bucket_size(&self) -> u32 {
        let logical_size = self.thumbnail_size.max(crate::ui::theme::THUMBNAIL_MIN) as u32;
        get_bucket_size(self.effective_thumbnail_request_size_px(logical_size))
    }

    pub(crate) fn effective_folder_preview_request_size_px(&self) -> u32 {
        let logical_size =
            (self.thumbnail_size.max(crate::ui::theme::THUMBNAIL_MIN) * 0.85).ceil() as u32;
        self.effective_thumbnail_request_size_px(logical_size)
    }

    pub(crate) fn current_folder_preview_bucket_size(&self) -> u32 {
        get_bucket_size(self.effective_folder_preview_request_size_px())
    }

    pub(crate) fn current_dynamic_texture_keep_count(&self) -> usize {
        if !self.thumbnail_caches_active() {
            return IDLE_THUMBNAIL_TEXTURE_KEEP;
        }

        dynamic_texture_keep_count(self.visible_grid_items_for_cache())
    }

    pub(crate) fn current_dynamic_folder_preview_keep_count(&self) -> usize {
        if !self.thumbnail_caches_active() {
            return IDLE_FOLDER_PREVIEW_KEEP;
        }

        dynamic_folder_preview_keep_count(self.visible_grid_items_for_cache())
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

    pub(crate) fn current_pending_thumbnail_upload_limit(&self) -> usize {
        if !self.thumbnail_caches_active() {
            return IDLE_PENDING_THUMBNAILS;
        }

        let bucket_size = self.current_thumbnail_bucket_size() as usize;
        let bucket_bytes = bucket_size
            .saturating_mul(bucket_size)
            .saturating_mul(4)
            .max(1);
        let byte_limited_items =
            (MAX_PENDING_THUMBNAIL_RGBA_BYTES / bucket_bytes).max(MIN_DYNAMIC_PENDING_THUMBNAILS);

        self.current_dynamic_texture_keep_count()
            .max(BASE_PENDING_THUMBNAILS)
            .min(MAX_DYNAMIC_PENDING_THUMBNAILS)
            .min(byte_limited_items)
    }

    pub(crate) fn trim_pending_thumbnail_uploads_to_limit(&mut self) {
        let max_pending = self.current_pending_thumbnail_upload_limit();
        if self.pending_thumbnails.len() <= max_pending {
            return;
        }

        let visible_paths = self.visible_grid_paths_snapshot();
        while self.pending_thumbnails.len() > max_pending {
            let evict_idx = visible_paths.as_ref().and_then(|visible_paths| {
                self.pending_thumbnails
                    .iter()
                    .position(|thumb| !visible_paths.contains(&thumb.path))
            });

            let old = if let Some(evict_idx) = evict_idx {
                self.pending_thumbnails.remove(evict_idx)
            } else {
                self.pending_thumbnails.pop_front()
            };

            if let Some(old) = old {
                self.cache_manager.finish_pending_upload(&old.path);
            } else {
                break;
            }
        }
    }

    fn trim_pending_thumbnail_uploads_to_count(&mut self, max_pending: usize) -> usize {
        let mut removed = 0usize;
        while self.pending_thumbnails.len() > max_pending {
            if let Some(old) = self.pending_thumbnails.pop_front() {
                self.cache_manager.finish_pending_upload(&old.path);
                removed += 1;
            } else {
                break;
            }
        }
        removed
    }

    pub fn log_memory_snapshot(&self, label: &str) {
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
        let rgba_target = self.current_dynamic_rgba_budget_bytes(DEFAULT_DYNAMIC_RGBA_BUDGET_BYTES);

        log::info!(
            "[MEM-TRACE:{label}] ws={:.1}MB private={:.1}MB items={} all_items={} tabs={} dir_cache={}/{} visible_items={} textures={}/{} texture_target={} folder_tex={}/{} folder_target={} rgba_items={} rgba={:.1}/{:.1}MB pending={} pending_rgba={:.1}MB pending_set={} loading={} folder_loading={} failed_thumbs={} queue={} vram_est={:.1}MB icons={} ext_icons={} drive_icons={} failed_drive_icons={} loading_drive_icons={} gifs={} gif_rgba={:.1}MB visible={:?} thumb_bucket={} folder_bucket={} frame_avg={:.1}ms frame_peak={:.1}ms upload_budget={:.1}ms",
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
            bytes_to_mb(pending_thumbnail_bytes as u64),
            self.cache_manager.pending_upload_set.len(),
            self.cache_manager.loading_set.len(),
            self.cache_manager.folder_preview_loading.len(),
            self.cache_manager.failed_thumbnails.len(),
            self.thumbnail_queue.pending_count(),
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

    fn run_memory_maintenance_impl(&mut self, force: bool) {
        if !force && self.last_memory_maintenance.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_memory_maintenance = Instant::now();

        let thumbnails_active = self.thumbnail_caches_active();
        if !thumbnails_active && !self.is_in_restore_burst() {
            let pending_removed = self.trim_pending_thumbnail_uploads_to_count(0);
            let (textures_removed, rgba_removed, folder_previews_removed) =
                self.cache_manager.trim_thumbnail_caches(
                    IDLE_THUMBNAIL_TEXTURE_KEEP,
                    IDLE_RGBA_BUDGET_BYTES,
                    IDLE_FOLDER_PREVIEW_KEEP,
                    None,
                );

            if textures_removed > 0
                || rgba_removed > 0
                || folder_previews_removed > 0
                || pending_removed > 0
            {
                log::debug!(
                    "[MEMORY] idle thumbnail trim textures={} rgba={} folder_previews={} pending={}",
                    textures_removed,
                    rgba_removed,
                    folder_previews_removed,
                    pending_removed,
                );
            }
        }

        let Some(process_memory) = current_process_memory_snapshot() else {
            return;
        };
        let working_set_bytes = process_memory.working_set_bytes;

        const SOFT_LIMIT_BYTES: u64 = 550 * 1024 * 1024;
        const HARD_LIMIT_BYTES: u64 = 700 * 1024 * 1024;

        if working_set_bytes < SOFT_LIMIT_BYTES {
            return;
        }

        let aggressive = working_set_bytes >= HARD_LIMIT_BYTES;
        let is_burst = self.is_in_restore_burst();
        let visible_grid_items = self.visible_grid_items_for_cache();
        let visible_paths = self.visible_grid_paths_snapshot();
        let texture_keep = self.current_dynamic_texture_keep_count();
        let folder_preview_keep = self.current_dynamic_folder_preview_keep_count();
        let rgba_budget = self.current_dynamic_rgba_budget_bytes(DEFAULT_DYNAMIC_RGBA_BUDGET_BYTES);
        let max_pending = self.current_pending_thumbnail_upload_limit();

        while self.pending_thumbnails.len() > max_pending {
            let evict_idx = visible_paths.as_ref().and_then(|visible_paths| {
                self.pending_thumbnails
                    .iter()
                    .position(|thumb| !visible_paths.contains(&thumb.path))
            });

            let old = if let Some(evict_idx) = evict_idx {
                self.pending_thumbnails.remove(evict_idx)
            } else {
                self.pending_thumbnails.pop_front()
            };

            if let Some(old) = old {
                self.cache_manager.finish_pending_upload(&old.path);
            } else {
                break;
            }
        }

        let (textures_removed, rgba_removed, folder_previews_removed) = if is_burst && !aggressive {
            // Skip texture/RGBA trimming during burst — we need the caches full.
            (0, 0, 0)
        } else if aggressive {
            self.cache_manager.trim_thumbnail_caches(
                texture_keep.max(96),
                dynamic_rgba_budget_bytes(
                    visible_grid_items,
                    self.current_thumbnail_bucket_size(),
                    MIN_RGBA_BUDGET_BYTES,
                ),
                folder_preview_keep.max(72),
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

        // Reuse existing GIF cleanup policy (TTL + bounded memory) without forcing visible preview drop.
        self.gif_manager.cleanup(false);

        if textures_removed > 0 || rgba_removed > 0 || folder_previews_removed > 0 {
            log::debug!(
                "[MEMORY] RAM {:.1}MB -> trimmed textures={} rgba={} folder_previews={} pending={} mode={}",
                working_set_bytes as f64 / 1024.0 / 1024.0,
                textures_removed,
                rgba_removed,
                folder_previews_removed,
                max_pending,
                if aggressive { "hard" } else { "soft" }
            );
        }
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

    pub(crate) fn visible_grid_paths_snapshot(&self) -> Option<FxHashSet<std::path::PathBuf>> {
        let mut visible_paths = FxHashSet::default();

        if matches!(self.view_mode, ViewMode::Grid) {
            insert_visible_paths_from_range(
                &mut visible_paths,
                self.items.as_ref().as_slice(),
                self.visible_index_range,
            );
        }

        if self.dual_panel_enabled {
            if let Some(snapshot) = self.dual_panel_inactive_state.as_ref() {
                if matches!(snapshot.view_mode, ViewMode::Grid) {
                    insert_visible_paths_from_range(
                        &mut visible_paths,
                        visible_items_for_snapshot(snapshot),
                        snapshot.visible_index_range,
                    );
                }
            }
        }

        (!visible_paths.is_empty()).then_some(visible_paths)
    }
}

pub(crate) fn dynamic_texture_keep_count(visible_grid_items: usize) -> usize {
    let target = visible_grid_items.saturating_mul(3).saturating_add(1) / 2;

    target
        .max(MIN_DYNAMIC_TEXTURE_CACHE_ITEMS)
        .min(MAX_DYNAMIC_TEXTURE_CACHE_ITEMS)
}

pub(crate) fn dynamic_folder_preview_keep_count(visible_grid_items: usize) -> usize {
    let target = visible_grid_items.saturating_mul(3).saturating_add(1) / 2;

    target
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
