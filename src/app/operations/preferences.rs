//! User preferences save/load
//!
//! This module handles saving application state to the SQLite database.
//!
//! PERFORMANCE: `save_preferences()` is debounced — it sets a dirty flag and the
//! actual write happens in `flush_preferences_if_needed()` which runs once per frame
//! but only writes to disk if >1 second has passed since the last write. This prevents
//! 20+ synchronous SQLite writes from blocking the UI thread on state changes.

use crate::app::navigation_state::ThemeMode;
use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::{FoldersPosition, SortMode, ViewMode};
use crate::domain::special_paths::is_virtual_path;
use crate::infrastructure::diagnostic_logger;
use std::time::SystemTime;

/// Minimum interval between actual disk writes
const PREFERENCES_FLUSH_INTERVAL_MS: u64 = 1000;

impl ImageViewerApp {
    /// Marks preferences as dirty (deferred write).
    /// The actual SQLite writes happen in `flush_preferences_if_needed()`.
    pub fn save_preferences(&mut self) {
        self.preferences_dirty = true;
    }

    /// Flushes dirty preferences to SQLite if enough time has passed.
    /// Called once per frame from the update loop.
    pub fn flush_preferences_if_needed(&mut self) {
        if !self.preferences_dirty {
            return;
        }
        if self.preferences_last_save.elapsed().as_millis() < PREFERENCES_FLUSH_INTERVAL_MS as u128
        {
            return;
        }
        // Non-blocking flush: if DB writer is busy, keep dirty=true and retry next frame.
        if self.do_save_preferences_nonblocking() {
            self.preferences_dirty = false;
            self.preferences_last_save = std::time::Instant::now();
        }
    }

    /// Force-flushes preferences immediately (for exit).
    pub fn force_save_preferences(&self) {
        self.do_save_preferences_blocking();
    }

    fn collect_preferences(&self) -> Vec<(&'static str, String)> {
        let mut prefs: Vec<(&'static str, String)> = Vec::with_capacity(48);

        // Always save the "normal" (unlocked) values so that locked-folder
        // overrides don't corrupt the persisted global preferences.
        let sort_mode_str = match self.sort_mode_normal {
            SortMode::Name => "name",
            SortMode::Date => "date",
            SortMode::Size => "size",
            SortMode::Type => "type",
            SortMode::DriveTotalSpace => "drive_total",
            SortMode::DriveFreeSpace => "drive_free",
        };
        prefs.push(("sort_mode", sort_mode_str.to_string()));

        let sort_mode_computer_str = match self.sort_mode_computer {
            SortMode::Name => "name",
            SortMode::DriveTotalSpace => "drive_total",
            SortMode::DriveFreeSpace => "drive_free",
            _ => "name", // Computer view only supports these 3
        };
        prefs.push(("sort_mode_computer", sort_mode_computer_str.to_string()));

        let sort_mode_normal_str = match self.sort_mode_normal {
            SortMode::Name => "name",
            SortMode::Date => "date",
            SortMode::Size => "size",
            SortMode::Type => "type",
            _ => "name", // Normal folders don't use drive modes
        };
        prefs.push(("sort_mode_normal", sort_mode_normal_str.to_string()));

        prefs.push((
            "sort_descending",
            (if self.sort_descending_normal {
                "true"
            } else {
                "false"
            })
            .to_string(),
        ));

        let folders_pos_str = match self.folders_position_normal {
            FoldersPosition::First => "first",
            FoldersPosition::Last => "last",
            FoldersPosition::Mixed => "mixed",
        };
        prefs.push(("folders_position", folders_pos_str.to_string()));

        // UI preferences
        prefs.push(("thumbnail_size", self.thumbnail_size.to_string()));

        let view_mode_str = match self.view_mode_normal {
            ViewMode::Grid => "grid",
            ViewMode::List => "list",
        };
        prefs.push(("view_mode", view_mode_str.to_string()));

        prefs.push((
            "show_left_sidebar",
            (if self.show_left_sidebar {
                "true"
            } else {
                "false"
            })
            .to_string(),
        ));

        prefs.push((
            "show_preview_panel",
            (if self.show_preview_panel {
                "true"
            } else {
                "false"
            })
            .to_string(),
        ));
        prefs.push((
            "show_recycle_bin",
            (if self.show_recycle_bin {
                "true"
            } else {
                "false"
            })
            .to_string(),
        ));
        prefs.push(("upload_budget_ms", self.upload_budget_ms.to_string()));

        // Window state persistence
        prefs.push(("window_width", self.layout.saved_window_width.to_string()));
        prefs.push(("window_height", self.layout.saved_window_height.to_string()));
        prefs.push((
            "window_is_maximized",
            (if self.layout.saved_is_maximized {
                "true"
            } else {
                "false"
            })
            .to_string(),
        ));

        // Sidebar widths persistence - only save valid values
        let left_to_save = self.layout.sidebar_left_width.max(150.0);
        let right_to_save = self.layout.sidebar_right_width.max(250.0);
        prefs.push(("sidebar_left_width", left_to_save.to_string()));
        prefs.push(("sidebar_right_width", right_to_save.to_string()));

        // Save last active folder from current tab
        let last_folder = self.tab_manager.active().path.clone();
        // Save if it's a real path or "This PC" (but not other virtual views or shell URIs)
        if !last_folder.is_empty()
            && !last_folder.starts_with("shell:")
            && (last_folder == crate::domain::special_paths::COMPUTER_VIEW_ID
                || !is_virtual_path(&last_folder))
        {
            prefs.push(("last_folder", last_folder));
        }

        // Save session volume (always available, independent of active player)
        prefs.push(("media_volume", self.session_volume.to_string()));

        // Show hidden files toggle
        prefs.push((
            "show_hidden_files",
            (if self.show_hidden_files {
                "true"
            } else {
                "false"
            })
            .to_string(),
        ));

        // Language preference
        prefs.push(("language", rust_i18n::locale().to_string()));

        // Theme preference
        let theme_str = match self.theme_mode {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        };
        prefs.push(("theme_mode", theme_str.to_string()));

        // GPU backend preference
        prefs.push(("gpu_backend", self.gpu_backend_preference.clone()));

        // Diagnostic mode preference
        prefs.push((
            diagnostic_logger::DIAGNOSTIC_MODE_KEY,
            if self.diagnostic_mode {
                "true".to_string()
            } else {
                "false".to_string()
            },
        ));
        prefs.push((
            diagnostic_logger::DIAGNOSTIC_MODE_ENABLED_AT_KEY,
            diagnostic_logger::format_enabled_at_preference(self.diagnostic_mode_enabled_at)
                .unwrap_or_default(),
        ));

        // Configurable keyboard shortcuts
        self.shortcuts.append_preferences(&mut prefs);

        // Save list view column widths - Regular view
        prefs.push((
            "list_col_name_width",
            self.layout.list_col_name_width.to_string(),
        ));
        prefs.push((
            "list_col_date_width",
            self.layout.list_col_date_width.to_string(),
        ));
        prefs.push((
            "list_col_type_width",
            self.layout.list_col_type_width.to_string(),
        ));
        prefs.push((
            "list_col_size_width",
            self.layout.list_col_size_width.to_string(),
        ));
        // Save list view column widths - OneDrive view
        prefs.push((
            "list_col_onedrive_name_width",
            self.layout.list_col_onedrive_name_width.to_string(),
        ));
        prefs.push((
            "list_col_onedrive_date_width",
            self.layout.list_col_onedrive_date_width.to_string(),
        ));
        prefs.push((
            "list_col_onedrive_type_width",
            self.layout.list_col_onedrive_type_width.to_string(),
        ));
        prefs.push((
            "list_col_onedrive_size_width",
            self.layout.list_col_onedrive_size_width.to_string(),
        ));
        prefs.push((
            "list_col_onedrive_status_width",
            self.layout.list_col_onedrive_status_width.to_string(),
        ));
        // Save list view column widths - Computer view
        prefs.push((
            "list_col_computer_name_width",
            self.layout.list_col_computer_name_width.to_string(),
        ));
        prefs.push((
            "list_col_computer_total_width",
            self.layout.list_col_computer_total_width.to_string(),
        ));
        prefs.push((
            "list_col_computer_free_width",
            self.layout.list_col_computer_free_width.to_string(),
        ));

        prefs
    }

    /// Non-blocking write attempt used by frame loop flush.
    fn do_save_preferences_nonblocking(&self) -> bool {
        let prefs = self.collect_preferences();
        self.app_state_db.try_set_preferences_batch(&prefs)
    }

    /// Blocking write used on exit to maximize persistence reliability.
    fn do_save_preferences_blocking(&self) {
        let prefs = self.collect_preferences();
        self.app_state_db.set_preferences_batch(&prefs);
    }

    pub fn set_diagnostic_mode(&mut self, enabled: bool) {
        self.set_diagnostic_mode_with_reason(enabled, "manual");
    }

    fn set_diagnostic_mode_with_reason(&mut self, enabled: bool, disable_reason: &'static str) {
        let logger_matches_state =
            self.diagnostic_mode == enabled && (!enabled || diagnostic_logger::is_enabled());
        if logger_matches_state {
            return;
        }

        if enabled {
            let enabled_since = self.diagnostic_mode_enabled_at.unwrap_or_else(SystemTime::now);
            match diagnostic_logger::enable_file_logging_with_since(enabled_since) {
                Ok(_) => {
                    self.diagnostic_mode = true;
                    self.diagnostic_mode_enabled_at = Some(enabled_since);
                    log::info!("[DIAGNOSTIC] Diagnostic mode enabled");
                    diagnostic_logger::diag_info(
                        "diagnostic_mode",
                        "enabled",
                        &[diagnostic_logger::field_label("activation", "manual")],
                    );
                }
                Err(error) => {
                    self.diagnostic_mode = false;
                    self.diagnostic_mode_enabled_at = None;
                    log::error!("[DIAGNOSTIC] Failed to enable diagnostic logging: {}", error);
                }
            }
        } else {
            if diagnostic_logger::is_enabled() {
                diagnostic_logger::diag_info(
                    "diagnostic_mode",
                    "disabled",
                    &[diagnostic_logger::field_label("reason", disable_reason)],
                );
                log::info!("[DIAGNOSTIC] Diagnostic mode disabled");
            }
            diagnostic_logger::disable_file_logging();
            self.diagnostic_mode = false;
            self.diagnostic_mode_enabled_at = None;
        }

        self.save_preferences();
        self.force_save_preferences();
    }

    pub fn auto_disable_diagnostic_mode_if_needed(&mut self) {
        if !self.diagnostic_mode {
            return;
        }

        if diagnostic_logger::is_preference_expired(self.diagnostic_mode_enabled_at, SystemTime::now())
        {
            log::info!("[DIAGNOSTIC] Auto-disabling diagnostic mode after 24 hours");
            self.set_diagnostic_mode_with_reason(false, "expired_24h");
        }
    }

    pub fn open_diagnostic_log_folder(&mut self) {
        if let Err(error) = diagnostic_logger::open_log_folder() {
            diagnostic_logger::diag_warn("diagnostic_mode", "open_log_folder_failed", &[]);
            log::warn!("[DIAGNOSTIC] Failed to open diagnostic log folder: {}", error);
            self.notifications.warning(error);
        }
    }
}
