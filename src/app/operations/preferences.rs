//! User preferences save/load
//!
//! This module handles saving application state to the SQLite database.
//!
//! PERFORMANCE: the frame loop submits debounced snapshots to a dedicated
//! writer. Lock and disk waits happen only there, never on the UI thread.

use crate::app::dual_panel::ActivePanel;
use crate::app::navigation_state::QuickAccessPlacement;
use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::{FoldersPosition, ViewMode};
use crate::domain::special_paths::{is_tag_view_path, is_virtual_path, COMPUTER_VIEW_ID};
use crate::infrastructure::diagnostic_logger;
use std::time::SystemTime;

pub(crate) mod writer;

fn view_mode_preference_value(view_mode: ViewMode) -> &'static str {
    view_mode.preference_value()
}

/// Minimum interval between debounced snapshot submissions.
const PREFERENCES_FLUSH_INTERVAL_MS: u64 = 1000;
const FINAL_PREFERENCES_FLUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

impl ImageViewerApp {
    /// Marks preferences as dirty (deferred write).
    /// The frame loop submits the snapshot later; a worker persists it.
    pub fn save_preferences(&mut self) {
        self.preferences_dirty = true;
    }

    /// Flushes dirty preferences to SQLite if enough time has passed.
    /// Called once per frame from the update loop.
    pub fn flush_preferences_if_needed(&mut self) {
        self.columns.folder_widths.poll_pending();
        if self
            .preference_writer
            .as_ref()
            .is_some_and(|writer| !writer.is_running())
        {
            self.preference_writer = None;
            self.preferences_dirty = true;
        }
        if self
            .preference_writer
            .as_ref()
            .is_some_and(|writer| writer.take_failure())
        {
            self.preferences_dirty = true;
            log::warn!("[PREFERENCES] Background write failed; retrying latest state");
        }
        if !self.preferences_dirty {
            return;
        }
        if self.preferences_last_save.elapsed().as_millis() < PREFERENCES_FLUSH_INTERVAL_MS as u128
        {
            return;
        }
        self.enqueue_preferences();
    }

    /// Submits final preferences and waits at most 250 ms (for exit).
    pub fn force_save_preferences(&mut self) -> bool {
        let deadline = std::time::Instant::now() + FINAL_PREFERENCES_FLUSH_TIMEOUT;
        let id = self.enqueue_preferences();
        let persisted = id.is_some_and(|id| {
            self.preference_writer.as_ref().is_some_and(|writer| {
                writer.wait(
                    id,
                    deadline.saturating_duration_since(std::time::Instant::now()),
                )
            })
        });
        let widths_persisted = self
            .columns
            .folder_widths
            .flush(deadline.saturating_duration_since(std::time::Instant::now()));
        if !persisted {
            self.preferences_dirty = true;
        }
        persisted && widths_persisted
    }

    /// Immediate submission, not immediate disk I/O. Settings changes use this;
    /// only shutdown/update paths should wait for persistence.
    pub fn save_preferences_async(&mut self) {
        self.enqueue_preferences();
    }

    fn enqueue_preferences(&mut self) -> Option<u64> {
        self.sync_to_tab();
        self.preferences_last_save = std::time::Instant::now();
        self.preferences_dirty = true;
        if self
            .preference_writer
            .as_ref()
            .is_some_and(|writer| !writer.is_running())
        {
            self.preference_writer = None;
        }
        if self.preference_writer.is_none() {
            match writer::PreferenceWriter::new(self.app_state_db.clone()) {
                Ok(writer) => self.preference_writer = Some(writer),
                Err(_) => return None,
            }
        }
        let snapshot = self.collect_preferences();
        let id = self.preference_writer.as_mut()?.submit(snapshot)?;
        self.preferences_dirty = false;
        Some(id)
    }

    fn collect_preferences(&self) -> Vec<(&'static str, String)> {
        let mut prefs: Vec<(&'static str, String)> = Vec::with_capacity(50);
        prefs.push((
            "startup_location",
            self.startup_location.preference_value().to_owned(),
        ));
        prefs.push(("startup_custom_path", self.startup_custom_path.clone()));
        prefs.push((
            "new_tab_location",
            self.tab_preferences.location.preference_value().to_owned(),
        ));
        prefs.push((
            "new_tab_custom_path",
            self.tab_preferences.custom_path.clone(),
        ));
        prefs.push((
            "archives_in_new_tab",
            self.tab_preferences.archives_in_new_tab.to_string(),
        ));
        prefs.push((
            "date_format",
            crate::domain::date_format::date_format()
                .preference_value()
                .to_owned(),
        ));

        // Always save the "normal" (unlocked) values so that locked-folder
        // overrides don't corrupt the persisted global preferences.
        prefs.push(("sort_mode", self.sort_mode_normal.preference_value()));
        prefs.push((
            "sort_mode_computer",
            self.sort_mode_computer.preference_value(),
        ));
        prefs.push(("sort_mode_normal", self.sort_mode_normal.preference_value()));
        if let Ok(columns) = serde_json::to_string(&self.columns.preferences) {
            prefs.push(("list_columns", columns));
        }

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
        prefs.push((
            "group_mode",
            self.group_mode_normal.preference_value().to_string(),
        ));
        prefs.push(("group_descending", self.group_descending_normal.to_string()));

        // UI preferences
        prefs.push(("thumbnail_size", self.thumbnail_size.to_string()));

        prefs.push((
            "view_mode",
            view_mode_preference_value(self.view_mode_normal).to_string(),
        ));

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
        prefs.push((
            "show_quick_access",
            (if self.show_quick_access {
                "true"
            } else {
                "false"
            })
            .to_string(),
        ));
        prefs.push((
            "minimize_to_tray",
            if self.minimize_to_tray {
                "true"
            } else {
                "false"
            }
            .to_string(),
        ));
        prefs.extend(self.explorer_preferences.preferences());
        prefs.push((
            "quick_access_placement",
            match self.quick_access_placement {
                QuickAccessPlacement::Sidebar => "sidebar",
                QuickAccessPlacement::Horizontal => "horizontal",
            }
            .to_string(),
        ));
        prefs.push((
            "show_tags",
            (if self.show_tags { "true" } else { "false" }).to_string(),
        ));
        prefs.push(("upload_budget_ms", self.upload_budget_ms.to_string()));
        prefs.push((
            "active_tag_filter",
            self.active_tag_filter
                .map(|id| id.to_string())
                .unwrap_or_default(),
        ));

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

        // Dual panel state persistence
        prefs.push((
            "dual_panel_enabled",
            (if self.dual_panel_enabled {
                "true"
            } else {
                "false"
            })
            .to_string(),
        ));
        prefs.push((
            "dual_panel_active",
            match self.dual_panel_active {
                ActivePanel::Left => "left",
                ActivePanel::Right => "right",
            }
            .to_string(),
        ));
        prefs.push((
            "dual_panel_split_ratio",
            self.layout.dual_panel_split_ratio.to_string(),
        ));
        prefs.push((
            "dual_panel_active_view_mode",
            view_mode_preference_value(self.view_mode).to_string(),
        ));

        // Persist inactive panel's path so it restores to its own folder.
        // Always write the key: preference batches only upsert, so omitting it
        // would leave an old inactive path in the database.
        let inactive_path = self
            .dual_panel_inactive_state
            .as_ref()
            .map(|snapshot| snapshot.path.as_str())
            .filter(|path| {
                !path.is_empty()
                    && !path.starts_with("shell:")
                    && (*path == COMPUTER_VIEW_ID
                        || is_tag_view_path(path)
                        || !is_virtual_path(path))
            })
            .unwrap_or_default();
        prefs.push(("dual_panel_inactive_path", inactive_path.to_string()));
        let inactive_view_mode = self
            .dual_panel_inactive_state
            .as_ref()
            .map(|snapshot| snapshot.view_mode)
            .unwrap_or(self.view_mode);
        prefs.push((
            "dual_panel_inactive_view_mode",
            view_mode_preference_value(inactive_view_mode).to_string(),
        ));

        match crate::tabs::TabManagerSession::from_tab_manager(&self.tab_manager).to_json() {
            Ok(session) => prefs.push(("tab_session", session)),
            Err(error) => log::warn!("[PREFERENCES] Could not serialize open tabs: {error}"),
        }

        // Sidebar widths persistence - only save valid values
        let left_to_save = self.layout.sidebar_left_width.max(150.0);
        let right_to_save = self.layout.sidebar_right_width.max(250.0);
        prefs.push(("sidebar_left_width", left_to_save.to_string()));
        prefs.push(("sidebar_right_width", right_to_save.to_string()));

        prefs.push((
            "sidebar_quick_access_height",
            self.layout
                .sidebar_quick_access_height
                .map(|height| height.to_string())
                .unwrap_or_default(),
        ));
        // Section collapse flags restart expanded. Do not persist a temporary
        // collapsed layout's limit as the next session's expanded height.
        let drives_height =
            if let Some(saved) = self.layout.sidebar_drives_height_before_collapse_all {
                saved
            } else if self.collapse_cloud_drives
                || self.collapse_local_disks
                || self.collapse_network_drives
                || self.collapse_network_locations
            {
                self.layout
                    .sidebar_drives_heights
                    .get(&[false; 4])
                    .copied()
                    .unwrap_or(self.layout.sidebar_drives_height)
            } else {
                self.layout.sidebar_drives_height
            };
        prefs.push((
            "sidebar_drives_height",
            drives_height
                .map(|height| height.to_string())
                .unwrap_or_default(),
        ));
        prefs.push((
            "sidebar_tag_order",
            crate::app::operations::tag_ops::serialize_sidebar_tag_order(&self.sidebar_tag_ids),
        ));

        // Save last active folder from current tab
        let last_folder = self.navigation_state.current_path.clone();
        // Save if it's a real path or "This PC" (but not other virtual views or shell URIs)
        if !last_folder.is_empty()
            && !last_folder.starts_with("shell:")
            && (last_folder == COMPUTER_VIEW_ID || !is_virtual_path(&last_folder))
        {
            prefs.push(("last_folder", last_folder));
        }

        // The player persists media_volume itself. Never include the main
        // window's cached value in an asynchronous snapshot: the player could
        // open and save a newer volume before that snapshot reaches the disk.

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
        let theme_str = self.theme_mode.preference();
        prefs.push(("theme_mode", theme_str.to_string()));
        prefs.push((
            crate::ui::list_stripes::PREFERENCE_KEY,
            self.list_stripes_enabled.to_string(),
        ));
        prefs.push((
            crate::domain::folder_appearance::FolderAppearance::PREFERENCE_KEY,
            self.folder_appearance.preference().to_string(),
        ));
        prefs.push((
            crate::ui::interface_scale::PREFERENCE_KEY,
            self.interface_scale.to_string(),
        ));

        // GPU backend preference
        prefs.push(("gpu_backend", self.gpu_backend_preference.clone()));

        prefs.push((
            "scroll_interpolation",
            if self.scroll_interpolation_enabled {
                "true".to_string()
            } else {
                "false".to_string()
            },
        ));

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

    pub fn set_diagnostic_mode(&mut self, enabled: bool) {
        self.set_diagnostic_mode_with_reason(enabled, "manual");
    }

    fn set_diagnostic_mode_with_reason(&mut self, enabled: bool, disable_reason: &'static str) {
        // The settings checkbox updates `self.diagnostic_mode` before calling
        // this method, so that field alone cannot tell whether the logger still
        // needs to transition. Compare the requested value with the logger's
        // actual state so a disable request closes the active file.
        let logger_matches_state = diagnostic_logger::is_enabled() == enabled;
        if logger_matches_state {
            return;
        }

        if enabled {
            let enabled_since = self
                .diagnostic_mode_enabled_at
                .unwrap_or_else(SystemTime::now);
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
                    log::error!(
                        "[DIAGNOSTIC] Failed to enable diagnostic logging: {}",
                        error
                    );
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
        self.save_preferences_async();
    }

    pub fn auto_disable_diagnostic_mode_if_needed(&mut self) {
        if diagnostic_logger::take_output_failure() {
            self.notifications.warning(
                "Diagnostic logging encountered an output error. Check disk space and permissions.",
            );
        }
        if !self.diagnostic_mode {
            return;
        }

        if diagnostic_logger::is_preference_expired(
            self.diagnostic_mode_enabled_at,
            SystemTime::now(),
        ) {
            log::info!("[DIAGNOSTIC] Auto-disabling diagnostic mode after 24 hours");
            self.set_diagnostic_mode_with_reason(false, "expired_24h");
        }
    }

    pub fn open_diagnostic_log_folder(&mut self) {
        if let Err(error) = diagnostic_logger::open_log_folder() {
            diagnostic_logger::diag_warn("diagnostic_mode", "open_log_folder_failed", &[]);
            log::warn!(
                "[DIAGNOSTIC] Failed to open diagnostic log folder: {}",
                error
            );
            self.notifications.warning(error);
        }
    }
}
