use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::domain::file_entry::FileEntry;
use crate::domain::file_entry::ViewMode;

use super::ImageViewerApp;

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

        let Some(working_set_bytes) = current_working_set_bytes() else {
            return;
        };

        const SOFT_LIMIT_BYTES: u64 = 550 * 1024 * 1024;
        const HARD_LIMIT_BYTES: u64 = 700 * 1024 * 1024;

        if working_set_bytes < SOFT_LIMIT_BYTES {
            return;
        }

        let aggressive = working_set_bytes >= HARD_LIMIT_BYTES;
        // During restore burst, allow a larger pending queue — we're actively
        // re-populating VRAM after an OS paging event.
        let is_burst = self.is_in_restore_burst();
        let max_pending = if is_burst {
            192
        } else if aggressive {
            24
        } else {
            48
        };
        let min_folder_previews_keep = self.estimated_visible_folder_previews();

        while self.pending_thumbnails.len() > max_pending {
            if let Some(old) = self.pending_thumbnails.pop_front() {
                self.cache_manager.finish_pending_upload(&old.path);
            } else {
                break;
            }
        }

        let (textures_removed, rgba_removed, folder_previews_removed) = if is_burst {
            // Skip texture/RGBA trimming during burst — we need the caches full.
            (0, 0, 0)
        } else if aggressive {
            self.cache_manager.trim_thumbnail_caches(
                96,
                64 * 1024 * 1024,
                min_folder_previews_keep.max(72),
            )
        } else {
            self.cache_manager.trim_thumbnail_caches(
                140,
                96 * 1024 * 1024,
                min_folder_previews_keep.max(120),
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

    fn estimated_visible_folder_previews(&self) -> usize {
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

        cols.saturating_mul(rows.saturating_add(2)).clamp(48, 320)
    }
}

#[cfg(target_os = "windows")]
fn current_working_set_bytes() -> Option<u64> {
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
            Some(counters.WorkingSetSize as u64)
        } else {
            None
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn current_working_set_bytes() -> Option<u64> {
    None
}
