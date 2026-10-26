//! Tab synchronization
//!
//! This module handles syncing state between the active tab and the main application state.

use crate::app::state::ImageViewerApp;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::Arc;

mod column_autofit;
mod staleness;
mod sync_to;

impl ImageViewerApp {
    /// Syncs the active tab state to the app
    pub fn sync_from_tab(&mut self) {
        self.cancel_sidebar_edits();
        self.context_menu.close();
        let sync_start = std::time::Instant::now();
        self.invalidate_active_items_rebuild();
        let previous_path = self.navigation_state.current_path.clone();
        let previous_is_computer_view = self.navigation_state.is_computer_view;
        let previous_is_recycle_bin_view = self.navigation_state.is_recycle_bin_view;
        let previous_dual_panel_enabled = self.dual_panel_enabled;
        let source_tab_id = self.tab_manager.active().id;
        let source_tab_items_len = self.tab_manager.active().visible_items_len();
        let source_tab_all_items_len = self.tab_manager.active().all_items.len();
        let source_tab_selection_len = self.tab_manager.active().multi_selection.len();
        let mut confirmed_folder_cover_removals = Vec::new();
        let mut deferred_folder_cover_revalidations = Vec::new();

        if self.is_loading_folder
            && !crate::domain::special_paths::is_virtual_path(&self.navigation_state.current_path)
        {
            self.directory_dirty_registry
                .mark_dirty(Path::new(&self.navigation_state.current_path));
        }
        if let Some(snapshot) = self.dual_panel_inactive_state.as_ref().filter(|snapshot| {
            snapshot.is_loading_folder
                && !crate::domain::special_paths::is_virtual_path(&snapshot.path)
        }) {
            snapshot
                .folder_load_generation
                .fetch_add(1, AtomicOrdering::Relaxed);
            self.directory_dirty_registry
                .mark_dirty(Path::new(&snapshot.path));
        }

        {
            let active = self.tab_manager.active_mut();
            let panel_tabs = active.dual_panel_tabs.clone();
            self.navigation_state.current_path = active.path.clone();
            self.current_folder_modified_hint = active.current_folder_modified_hint.clone();
            self.current_folder_created_hint = active.current_folder_created_hint.clone();
            self.navigation_state.path_input = active.path_input.clone();
            self.navigation_state.is_computer_view = active.is_computer_view;
            self.navigation_state.is_recycle_bin_view = active.is_recycle_bin_view;
            self.navigation_state.navigation = active.navigation.clone();
            let restore_items_from_all_items = active.items_snapshot_compact;
            self.all_items = std::mem::take(&mut active.all_items);
            self.items = if restore_items_from_all_items {
                self.all_items.clone()
            } else {
                active.items.clone()
            };
            active.items_snapshot_compact = false;
            self.generation = active.generation;
            self.is_loading_folder = active.is_loading_folder;
            self.items_revision = active.items_revision;
            self.pending_items_rebuild = active.pending_items_rebuild;
            self.pending_items_count = active.pending_items_count;
            self.inactive_final_items_rebuild_pending = active.inactive_final_items_rebuild_pending;
            self.selected_item = active.selected_item;
            self.selected_file = active.selected_file.clone();

            // FIX: Validate that selected_file still exists in items
            // If the file was moved/deleted while on another tab, clear selection
            // so preview panel shows current folder info instead of stale data.
            // This is a pure in-memory check (no filesystem I/O).
            if let Some(ref selected) = self.selected_file {
                let still_exists = self.items.iter().any(|item| item.path == selected.path);
                if !still_exists {
                    // Store selected path before clearing for folder cover check
                    let removed_path = selected.path.clone();

                    self.selected_file = None;
                    self.selected_item = None;
                    self.selected_thumbnail = None;
                    self.selected_metadata = None;
                    // Also clear from tab so it doesn't come back on next sync
                    active.selected_file = None;
                    active.selected_item = None;
                    active.selected_thumbnail = None;
                    active.selected_metadata = None;

                    // FIX: If the removed file was the folder cover for this folder,
                    // invalidate the folder_preview_cache so the preview panel updates.
                    // Uses SQLite lookup (minimal I/O) and requests recalculation.
                    let current_folder = std::path::PathBuf::from(&active.path);
                    match self
                        .app_state_db
                        .try_get_folder_covers(std::slice::from_ref(&current_folder))
                    {
                        crate::infrastructure::app_state_db::FolderCoverReadOutcome::Completed(
                            covers,
                        ) => {
                            if covers.get(&current_folder) == Some(&removed_path) {
                                confirmed_folder_cover_removals.push(current_folder.clone());
                                self.cache_manager.folder_preview_cache.pop(&current_folder);
                            }
                        }
                        crate::infrastructure::app_state_db::FolderCoverReadOutcome::Busy
                        | crate::infrastructure::app_state_db::FolderCoverReadOutcome::Failed => {
                            deferred_folder_cover_revalidations.push(current_folder);
                        }
                    }
                }
            }

            self.selected_thumbnail = std::mem::take(&mut active.selected_thumbnail);
            self.selected_gif = std::mem::take(&mut active.selected_gif);
            self.selected_metadata = std::mem::take(&mut active.selected_metadata);
            self.search_query = std::mem::take(&mut active.search_query);
            self.scroll_to_selected = active.scroll_to_selected;
            self.scroll_offset_y = active.scroll_offset_y;
            self.scroll_offset_x = active.scroll_offset_x;
            self.total_items = active.total_items;
            self.view_mode = active.view_mode;
            self.multi_selection = std::mem::take(&mut active.multi_selection);
            self.sort_mode = active.sort_mode;
            self.sort_descending = active.sort_descending;
            self.folders_position = active.folders_position;
            self.group_mode = active.group_mode;
            self.group_descending = active.group_descending;
            self.group_projection = active.group_projection.clone();
            self.collapsed_groups_by_context = active.collapsed_groups_by_context.clone();
            self.show_left_sidebar = active.show_left_sidebar;
            self.show_preview_panel = active.show_preview_panel;
            self.collapse_quick_access = active.collapse_quick_access;
            self.collapse_cloud_drives = active.collapse_cloud_drives;
            self.collapse_local_disks = active.collapse_local_disks;
            self.collapse_network_drives = active.collapse_network_drives;
            self.collapse_network_locations = active.collapse_network_locations;
            self.active_tag_filter = active.active_tag_filter;
            self.collapse_tags = active.collapse_tags;

            // Restore dual panel state from tab
            self.dual_panel_enabled = active.dual_panel_enabled;
            self.dual_panel_active = active.dual_panel_active;
            self.layout.dual_panel_split_ratio = active.dual_panel_split_ratio;
            self.pending_list_column_autofit = column_autofit::pending_on_restore(
                previous_dual_panel_enabled,
                active.dual_panel_enabled,
                self.items.is_empty(),
                panel_tabs
                    .as_ref()
                    .and_then(|tabs| tabs.panel(self.dual_panel_active).active())
                    .is_some_and(|tab| tab.snapshot.pending_list_column_autofit),
            );
            if active.dual_panel_enabled {
                if let Some(widths) = active.dual_panel_active_list_column_widths {
                    widths.apply_to_layout(&mut self.layout);
                }
            }
            self.dual_panel_inactive_state =
                active.dual_panel_inactive_state.take().map(|mut snapshot| {
                    snapshot.restore_from_storage();
                    snapshot
                });
            if active.dual_panel_enabled {
                if let Some(snapshot) = panel_tabs
                    .as_ref()
                    .and_then(|tabs| tabs.panel(self.dual_panel_active.other()).active())
                    .map(|tab| tab.snapshot.clone())
                {
                    let mut snapshot = snapshot;
                    snapshot.restore_from_storage();
                    self.dual_panel_inactive_state = Some(snapshot);
                }
            }
        }

        for folder_path in confirmed_folder_cover_removals {
            self.remove_folder_cover_without_blocking(&folder_path, true);
        }
        for folder_path in deferred_folder_cover_revalidations {
            self.schedule_folder_cover_refresh(&folder_path);
        }

        let restored_path = self.navigation_state.current_path.clone();
        self.spawn_folder_meta_resolve_if_needed(Path::new(&restored_path), &restored_path);

        // Active-panel folder jobs do not survive a tab switch because their
        // streamed UI results are routed only to the current tab. Cancel the
        // outgoing token and give the restored tab a fresh one so old jobs
        // cannot become valid again after a later tab round-trip.
        self.folder_load_generation
            .fetch_add(1, AtomicOrdering::Relaxed);
        self.folder_load_generation = Arc::new(AtomicUsize::new(self.generation));
        self.sync_thumbnail_generation_gate();
        self.visible_group_paths.clear();

        // A tab's streamed load was cancelled when it stopped being current.
        // Restart it instead of exposing the partial snapshot saved above.
        let restored_load_in_progress = self.is_loading_folder;

        if self.navigation_state.current_path != previous_path
            || self.navigation_state.is_computer_view != previous_is_computer_view
            || self.navigation_state.is_recycle_bin_view != previous_is_recycle_bin_view
        {
            if self.dual_panel_enabled {
                self.prune_thumbnail_pipeline_for_dual_panel_navigation("tab-switch");
            } else {
                self.discard_thumbnail_pipeline_for_navigation("tab-switch", true);
            }
        }

        // Restore per-tab sidebar state (expanded nodes + scroll position)
        {
            let active = self.tab_manager.active();
            let sidebar_expanded = active.sidebar_expanded.clone();
            let sidebar_scroll_y = active.sidebar_scroll_y;
            self.sidebar_tree
                .restore_expanded(sidebar_expanded, sidebar_scroll_y);
        }

        // Apply folder lock if the destination tab's folder has locked preferences.
        // Use the tab-restore variant so that per-tab sort/view preferences are
        // preserved for unlocked folders (the full apply_folder_lock_if_present
        // would reset them to global "normal" defaults, discarding tab state).
        self.apply_folder_lock_on_tab_restore();

        // Restoration may have cleared a missing selection even when there is
        // no thumbnail work to prepare. Reconcile the owner's preview as well.
        self.close_embedded_preview_for_selection_change();

        if self.show_preview_panel && self.needs_selected_preview_preparation() {
            self.update_selected_thumbnail();
        }

        // Stored tabs redirected after eject have intentionally empty listings.
        // Computer views must rebuild from the current drive list on restore.
        if self.navigation_state.is_computer_view {
            self.setup_computer_view();
        }
        if self
            .dual_panel_inactive_state
            .as_ref()
            .is_some_and(|snapshot| snapshot.is_computer_view)
        {
            self.with_inactive_panel(|app| app.setup_computer_view());
        }
        self.watch_current_folder();

        // If items were cleared (by MoveCompleted event) and this is a regular folder view,
        // trigger a reload to fetch fresh content
        let is_virtual_path =
            crate::domain::special_paths::is_virtual_path(&self.navigation_state.current_path);
        let mut needs_reload = self.items.is_empty()
            && !self.navigation_state.is_computer_view
            && !self.navigation_state.is_recycle_bin_view
            && !is_virtual_path
            && !self.navigation_state.current_path.is_empty();

        if restored_load_in_progress && !self.navigation_state.is_computer_view {
            self.loaded_path.clear();
            if let Some(tag_id) = crate::domain::special_paths::tag_id_from_view_path(
                &self.navigation_state.current_path,
            ) {
                self.setup_tag_view(tag_id);
                needs_reload = false;
            } else if self.navigation_state.is_recycle_bin_view {
                self.setup_recycle_bin_view();
                needs_reload = false;
            } else {
                self.load_folder(false);
                needs_reload = false;
            }
        } else if self.items.is_empty() {
            if let Some(tag_id) = crate::domain::special_paths::tag_id_from_view_path(
                &self.navigation_state.current_path,
            ) {
                self.setup_tag_view(tag_id);
                needs_reload = false;
            } else if self.navigation_state.is_recycle_bin_view {
                // New background tabs have not loaded their virtual listing yet.
                self.setup_recycle_bin_view();
                needs_reload = false;
            }
        }

        let inactive_load_to_restart = self
            .dual_panel_inactive_state
            .as_ref()
            .is_some_and(|snapshot| snapshot.is_loading_folder);
        if inactive_load_to_restart {
            self.with_inactive_panel(|app| {
                app.loaded_path.clear();
                if let Some(tag_id) = crate::domain::special_paths::tag_id_from_view_path(
                    &app.navigation_state.current_path,
                ) {
                    app.setup_tag_view(tag_id);
                } else if app.navigation_state.is_recycle_bin_view {
                    app.setup_recycle_bin_view();
                } else if !app.navigation_state.is_computer_view
                    && !app.navigation_state.is_recycle_bin_view
                {
                    app.load_folder_for_inactive();
                }
            });
        } else {
            let inactive_tag_to_reload = self
                .dual_panel_inactive_state
                .as_ref()
                .filter(|snapshot| snapshot.items.is_empty() && snapshot.all_items.is_empty())
                .and_then(|snapshot| {
                    crate::domain::special_paths::tag_id_from_view_path(&snapshot.path)
                });
            if let Some(tag_id) = inactive_tag_to_reload {
                self.with_inactive_panel(|app| app.setup_tag_view(tag_id));
            }
        }

        let inactive_folder_to_restart = self
            .dual_panel_inactive_state
            .as_ref()
            .filter(|snapshot| {
                snapshot.is_loading_folder
                    && snapshot
                        .folder_load_generation
                        .load(AtomicOrdering::Relaxed)
                        != snapshot.generation
                    && !crate::domain::special_paths::is_virtual_path(&snapshot.path)
            })
            .map(|snapshot| PathBuf::from(&snapshot.path));
        if let Some(path) = inactive_folder_to_restart {
            self.reload_inactive_panel_if_matches(&[&path]);
        }

        // TAB-SWITCH STALENESS CHECK: Even when the tab has cached items,
        // verify the directory hasn't changed while the tab was inactive.
        // Without this, changes made externally (e.g., in Windows Explorer)
        // won't be visible until the consistency probe catches up (up to 30s).
        if !needs_reload
            && !self.items.is_empty()
            && !self.navigation_state.is_computer_view
            && !self.navigation_state.is_recycle_bin_view
            && !is_virtual_path
            && !self.navigation_state.current_path.is_empty()
        {
            let tab_path = std::path::PathBuf::from(&self.navigation_state.current_path);

            // Check the in-memory dirty registry first, then use the same
            // non-blocking/cloud-aware probe policy as panel-tab restoration.
            let (is_dirty, is_stale, service_checked) = self.tab_path_staleness(&tab_path);

            if is_stale {
                log::info!(
                    "[TAB] Tab-switch staleness detected for {:?}, scheduling reload (dirty={}, service_checked={})",
                    tab_path,
                    is_dirty,
                    service_checked
                );
                self.directory_dirty_registry.mark_dirty(&tab_path);
                self.directory_cache.invalidate(&tab_path);
                needs_reload = true;
            }
        }

        if needs_reload {
            log::debug!(
                "[TAB] Detected cleared items cache, reloading folder: {}",
                self.navigation_state.current_path
            );
            // Reset loaded_path to bypass the guard in load_folder
            self.loaded_path.clear();
            self.load_folder(false);
        }

        let sync_ms = sync_start.elapsed().as_millis();
        if sync_ms > 80 {
            let source_tab_path = self.tab_manager.active().path.clone();
            log::warn!(
                "[PERF-TAB] sync_from_tab total={}ms tab_id={} path={} src_items={} src_all_items={} src_selection={} app_items={} app_all_items={} app_selection={} needs_reload={}",
                sync_ms,
                source_tab_id,
                source_tab_path,
                source_tab_items_len,
                source_tab_all_items_len,
                source_tab_selection_len,
                self.items.len(),
                self.all_items.len(),
                self.multi_selection.len(),
                needs_reload,
            );
        }
    }
}
