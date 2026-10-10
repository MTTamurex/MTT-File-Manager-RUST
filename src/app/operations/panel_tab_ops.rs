use crate::app::dual_panel::{ActivePanel, PanelSnapshot};
use crate::app::state::ImageViewerApp;
use crate::application::navigation::NavigationHistory;
use crate::tabs::MAX_PANEL_TABS;
use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::Arc;

impl ImageViewerApp {
    fn new_panel_tab_snapshot(&mut self) -> PanelSnapshot {
        let mut snapshot = PanelSnapshot::from_app(self);
        let path = snapshot.path.clone();

        snapshot.path_input = path.clone();
        snapshot.navigation = NavigationHistory::new(path);
        snapshot.items = Arc::new(Vec::new());
        snapshot.all_items = Arc::new(Vec::new());
        snapshot.items_revision = snapshot.items_revision.wrapping_add(1);
        snapshot.items_snapshot_compact = false;
        snapshot.total_items = 0;
        snapshot.is_loading_folder = false;
        snapshot.folder_load_error = None;
        snapshot.selected_item = None;
        snapshot.selected_file = None;
        snapshot.multi_selection.clear();
        snapshot.selection_anchor = None;
        snapshot.rectangle_selection_state = None;
        snapshot.scroll_offset_y = 0.0;
        snapshot.scroll_offset_x = 0.0;
        snapshot.scroll_to_selected = false;
        snapshot.visible_index_range = None;
        snapshot.visible_paths_cache.clear();
        snapshot.visible_group_paths.clear();
        snapshot.visible_range_cached = None;
        snapshot.search_query.clear();
        snapshot.active_tag_filter = None;
        snapshot.selected_thumbnail = None;
        snapshot.selected_metadata = None;
        snapshot.selected_gif = None;
        snapshot.loaded_path.clear();
        snapshot.renaming_state = None;
        snapshot.focus_rename = false;
        snapshot.pending_all_items_clear = false;
        snapshot.hold_visible_items_until_load_complete = false;
        snapshot.pending_items_rebuild = false;
        snapshot.pending_items_count = 0;
        snapshot.inactive_final_items_rebuild_pending = false;
        snapshot.stale_items_snapshot = None;
        snapshot.generation =
            crate::app::operations::folder_loading::allocate_folder_load_generation();
        snapshot.folder_load_generation =
            Arc::new(std::sync::atomic::AtomicUsize::new(snapshot.generation));
        snapshot.current_generation = self.current_generation.clone();
        snapshot
    }

    fn restore_panel_tab_snapshot(&mut self, mut snapshot: PanelSnapshot) {
        self.context_menu.close();
        let old_path = self.navigation_state.current_path.clone();
        let restored_path = snapshot.path.clone();
        let restart_incomplete_load = snapshot.is_loading_folder;
        let current_outer_tab_id = self.tab_manager.active().id;
        let restored_path_is_dirty = self
            .directory_dirty_registry
            .is_dirty(&std::path::PathBuf::from(&restored_path));

        if self.media_preview_owner_tab_id == Some(current_outer_tab_id) {
            self.destroy_media_preview();
        }

        if self.is_loading_folder && !crate::domain::special_paths::is_virtual_path(&old_path) {
            self.directory_dirty_registry
                .mark_dirty(std::path::Path::new(&old_path));
        }
        self.folder_load_generation
            .fetch_add(1, AtomicOrdering::Relaxed);
        self.invalidate_active_items_rebuild();

        let shared_generation = self.current_generation.clone();
        snapshot
            .folder_load_generation
            .fetch_add(1, AtomicOrdering::Relaxed);
        snapshot.generation =
            crate::app::operations::folder_loading::allocate_folder_load_generation();
        snapshot.folder_load_generation =
            Arc::new(std::sync::atomic::AtomicUsize::new(snapshot.generation));
        snapshot.current_generation = shared_generation;
        snapshot.restore_from_storage();
        snapshot.is_loading_folder = false;
        snapshot.apply_to(self);
        self.sync_thumbnail_generation_gate();
        self.visible_group_paths.clear();

        let path_changed = self.navigation_state.current_path != old_path;
        if path_changed {
            if self.dual_panel_enabled {
                self.prune_thumbnail_pipeline_for_dual_panel_navigation("panel-tab-switch");
            } else {
                self.discard_thumbnail_pipeline_for_navigation("panel-tab-switch", true);
            }
        }

        self.apply_folder_lock_on_tab_restore();
        self.watch_current_folder();

        let needs_reload = restart_incomplete_load
            || restored_path_is_dirty
            || (self.items.is_empty() && self.all_items.is_empty());

        if needs_reload {
            self.loaded_path.clear();
            if self.navigation_state.is_computer_view {
                self.setup_computer_view();
            } else if self.navigation_state.is_recycle_bin_view {
                self.setup_recycle_bin_view();
            } else if let Some(tag_id) = crate::domain::special_paths::tag_id_from_view_path(
                &self.navigation_state.current_path,
            ) {
                self.setup_tag_view(tag_id);
            } else if !crate::domain::special_paths::is_virtual_path(
                &self.navigation_state.current_path,
            ) {
                self.load_folder(false);
            }
        }

        if self.show_preview_panel && self.needs_selected_preview_preparation() {
            self.update_selected_thumbnail();
        }
    }

    fn focus_panel_for_tab_action(&mut self, panel: ActivePanel) {
        if self.dual_panel_active != panel {
            self.dual_panel_switch_active();
        }
    }

    pub fn dual_panel_select_tab(&mut self, panel: ActivePanel, index: usize) -> bool {
        if !self.dual_panel_enabled {
            return false;
        }

        self.sync_to_tab();
        self.focus_panel_for_tab_action(panel);

        let target = {
            let Some(tabs) = self.tab_manager.active_mut().dual_panel_tabs.as_mut() else {
                return false;
            };
            let panel_tabs = tabs.panel_mut(panel);
            if !panel_tabs.select(index) {
                return true;
            }
            panel_tabs.active().map(|tab| tab.snapshot.clone())
        };
        let Some(target) = target else {
            return false;
        };

        self.restore_panel_tab_snapshot(target);
        self.sync_to_tab();
        self.update_video_visibility();
        self.save_preferences();
        true
    }

    pub fn dual_panel_add_tab(&mut self, panel: ActivePanel) -> bool {
        if !self.dual_panel_enabled {
            return false;
        }

        self.sync_to_tab();
        self.focus_panel_for_tab_action(panel);

        if !self
            .tab_manager
            .active()
            .dual_panel_tabs
            .as_ref()
            .is_some_and(|tabs| tabs.can_add_tab())
        {
            self.notifications
                .warning(rust_i18n::t!("tabs.max_reached", max = MAX_PANEL_TABS).to_string());
            return false;
        }

        let snapshot = self.new_panel_tab_snapshot();
        let index = {
            let tabs = self
                .tab_manager
                .active_mut()
                .dual_panel_tabs
                .as_mut()
                .expect("dual panel tabs are initialized when dual mode is active");
            tabs.add_tab(panel, snapshot)
        };
        let Some(index) = index else {
            return false;
        };

        let target = self
            .tab_manager
            .active()
            .dual_panel_tabs
            .as_ref()
            .and_then(|tabs| tabs.panel(panel).tabs.get(index))
            .map(|tab| tab.snapshot.clone());
        let Some(target) = target else {
            return false;
        };

        self.restore_panel_tab_snapshot(target);
        self.sync_to_tab();
        self.update_video_visibility();
        self.save_preferences();
        true
    }

    pub fn dual_panel_close_tab(&mut self, panel: ActivePanel, index: usize) -> bool {
        if !self.dual_panel_enabled {
            return false;
        }

        self.sync_to_tab();
        self.focus_panel_for_tab_action(panel);

        let was_active = self
            .tab_manager
            .active_mut()
            .dual_panel_tabs
            .as_mut()
            .and_then(|tabs| tabs.panel_mut(panel).close(index));
        let Some(was_active) = was_active else {
            return false;
        };

        if was_active {
            let target = self
                .tab_manager
                .active()
                .dual_panel_tabs
                .as_ref()
                .and_then(|tabs| tabs.panel(panel).active())
                .map(|tab| tab.snapshot.clone());
            if let Some(target) = target {
                self.restore_panel_tab_snapshot(target);
            }
        }

        self.sync_to_tab();
        self.update_video_visibility();
        self.save_preferences();
        true
    }
}
