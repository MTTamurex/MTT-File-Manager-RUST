//! Tab synchronization
//!
//! This module handles syncing state between the active tab and the main application state.

use crate::app::dual_panel::PanelListColumnWidths;
use crate::app::state::ImageViewerApp;
use crate::domain::special_paths::COMPUTER_VIEW_ID;
use std::path::Path;
use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::Arc;

impl ImageViewerApp {
    pub fn sync_to_tab(&mut self) {
        let current_tag_title =
            self.tag_view_display_name_for_path(&self.navigation_state.current_path);
        let active = self.tab_manager.active_mut();
        active.path = self.navigation_state.current_path.clone();
        active.path_input = self.navigation_state.path_input.clone();
        active.is_computer_view = self.navigation_state.is_computer_view;
        active.is_recycle_bin_view = self.navigation_state.is_recycle_bin_view;
        active.navigation = self.navigation_state.navigation.clone();
        let compact_items_snapshot = self.search_query.is_empty()
            && !self.is_loading_folder
            && !self.pending_items_rebuild
            && self.items.len() == self.all_items.len();
        if compact_items_snapshot {
            active.all_items = self.items.clone();
            active.items = Arc::new(Vec::new());
            active.items_snapshot_compact = true;
        } else {
            active.all_items = self.all_items.clone();
            active.items = self.items.clone();
            active.items_snapshot_compact = false;
        }
        active.selected_item = self.selected_item;
        active.generation = self.generation;
        active.selected_file = self.selected_file.clone();
        active.selected_thumbnail = None;
        active.selected_gif = None;
        active.selected_metadata = self.selected_metadata.clone();
        active.search_query = self.search_query.clone();
        active.scroll_to_selected = self.scroll_to_selected;
        active.scroll_offset_y = self.scroll_offset_y;
        active.scroll_offset_x = self.scroll_offset_x;
        active.total_items = self.total_items;
        active.view_mode = self.view_mode;
        // PERF: Move instead of clone for multi_selection (same pattern as all_items)
        active.multi_selection = std::mem::take(&mut self.multi_selection);
        active.sort_mode = self.sort_mode;
        active.sort_descending = self.sort_descending;
        active.folders_position = self.folders_position;
        active.show_left_sidebar = self.show_left_sidebar;
        active.show_preview_panel = self.show_preview_panel;
        active.collapse_quick_access = self.collapse_quick_access;
        active.collapse_cloud_drives = self.collapse_cloud_drives;
        active.collapse_local_disks = self.collapse_local_disks;
        active.collapse_network_drives = self.collapse_network_drives;
        active.active_tag_filter = self.active_tag_filter;
        active.collapse_tags = self.collapse_tags;

        // Save dual panel state per-tab
        active.dual_panel_enabled = self.dual_panel_enabled;
        active.dual_panel_active = self.dual_panel_active;
        active.dual_panel_split_ratio = self.layout.dual_panel_split_ratio;
        active.dual_panel_active_list_column_widths = self
            .dual_panel_enabled
            .then(|| PanelListColumnWidths::from_layout(&self.layout));
        active.dual_panel_inactive_state =
            self.dual_panel_inactive_state.clone().map(|mut snapshot| {
                snapshot.compact_for_storage();
                snapshot
            });

        // Save per-tab sidebar state (expanded nodes + scroll position)
        active.sidebar_expanded = self.sidebar_tree.snapshot_expanded();
        active.sidebar_scroll_y = self.sidebar_tree.snapshot_scroll_y();

        // On Windows, Path::new(COMPUTER_VIEW_ID).file_name() is None
        if active.is_computer_view {
            active.title = COMPUTER_VIEW_ID.to_string();
        } else if let Some(title) = current_tag_title {
            active.title = title;
        } else {
            active.title = Path::new(&active.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| active.path.clone());
        }
    }

    /// Syncs the active tab state to the app
    pub fn sync_from_tab(&mut self) {
        let sync_start = std::time::Instant::now();
        self.invalidate_active_items_rebuild();
        let previous_path = self.navigation_state.current_path.clone();
        let previous_is_computer_view = self.navigation_state.is_computer_view;
        let previous_is_recycle_bin_view = self.navigation_state.is_recycle_bin_view;
        let source_tab_id = self.tab_manager.active().id;
        let source_tab_items_len = self.tab_manager.active().visible_items_len();
        let source_tab_all_items_len = self.tab_manager.active().all_items.len();
        let source_tab_selection_len = self.tab_manager.active().multi_selection.len();

        {
            let active = self.tab_manager.active_mut();
            self.navigation_state.current_path = active.path.clone();
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
                    let covers = self
                        .app_state_db
                        .get_folder_covers(std::slice::from_ref(&current_folder));
                    if let Some(current_cover) = covers.get(&current_folder) {
                        if current_cover == &removed_path {
                            self.app_state_db.remove_folder_cover(&current_folder);
                            self.cache_manager.folder_preview_cache.pop(&current_folder);
                            let _ = self.cover_worker_sender.send(current_folder);
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
            self.show_left_sidebar = active.show_left_sidebar;
            self.show_preview_panel = active.show_preview_panel;
            self.collapse_quick_access = active.collapse_quick_access;
            self.collapse_cloud_drives = active.collapse_cloud_drives;
            self.collapse_local_disks = active.collapse_local_disks;
            self.collapse_network_drives = active.collapse_network_drives;
            self.active_tag_filter = active.active_tag_filter;
            self.collapse_tags = active.collapse_tags;

            // Restore dual panel state from tab
            self.dual_panel_enabled = active.dual_panel_enabled;
            self.dual_panel_active = active.dual_panel_active;
            self.layout.dual_panel_split_ratio = active.dual_panel_split_ratio;
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
        }

        self.current_generation
            .store(self.generation, AtomicOrdering::Relaxed);

        if self.navigation_state.current_path != previous_path
            || self.navigation_state.is_computer_view != previous_is_computer_view
            || self.navigation_state.is_recycle_bin_view != previous_is_recycle_bin_view
        {
            self.discard_thumbnail_pipeline_for_navigation("tab-switch", true);
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

        if self.show_preview_panel && self.needs_selected_preview_preparation() {
            self.update_selected_thumbnail();
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

        if self.items.is_empty() {
            if let Some(tag_id) = crate::domain::special_paths::tag_id_from_view_path(
                &self.navigation_state.current_path,
            ) {
                self.setup_tag_view(tag_id);
                needs_reload = false;
            }
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

            // 1) Check in-memory dirty registry (free, no I/O)
            let is_dirty = self.directory_dirty_registry.is_dirty(&tab_path);

            // 2) Fast-path for NTFS: ask the search service (no disk I/O).
            //    The service runs with admin privileges and tracks USN journal
            //    changes with dir_modified_at timestamps per directory FRN.
            //    Threshold of 120s covers any reasonable tab-away duration.
            let mut service_checked = false;
            let is_stale = if is_dirty {
                true
            } else if crate::infrastructure::onedrive::is_cloud_sync_path(&tab_path) {
                self.directory_cache
                    .cached_at_ms(&tab_path)
                    .map(|cached_at_ms| {
                        !crate::infrastructure::onedrive::directory_cache_is_recent(cached_at_ms)
                    })
                    .unwrap_or(true)
            } else if self.global_search.available {
                // Try the search service first (NTFS fast path, ~1-2ms via named pipe)
                let path_str = self.navigation_state.current_path.clone();
                match crate::infrastructure::global_search::check_paths_modified(&[path_str], 120) {
                    Ok(modified) => {
                        service_checked = true;
                        !modified.is_empty()
                    }
                    Err(e) => {
                        log::debug!(
                            "[TAB] Search service check_paths_modified failed; skipping UI-thread mtime fallback: {}",
                            e
                        );
                        false
                    }
                }
            } else if self.directory_cache.cached_at_ms(&tab_path).is_some() {
                // Never call std::fs::metadata() on the UI thread here. If the
                // tab path was deleted or the disk is waking up, metadata can
                // stall the whole app; watcher/consistency probes will catch up.
                false
            } else {
                // No cache entry at all — load_folder will handle it
                false
            };

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
