//! Tab synchronization
//!
//! This module handles syncing state between the active tab and the main application state.

use crate::app::state::ImageViewerApp;
use crate::domain::special_paths::COMPUTER_VIEW_ID;
use std::path::Path;

impl ImageViewerApp {
    pub fn sync_to_tab(&mut self) {
        let active = self.tab_manager.active_mut();
        active.path = self.navigation_state.current_path.clone();
        active.path_input = self.navigation_state.path_input.clone();
        active.is_computer_view = self.navigation_state.is_computer_view;
        active.is_recycle_bin_view = self.navigation_state.is_recycle_bin_view;
        active.navigation = self.navigation_state.navigation.clone();
        active.items = self.items.clone();
        // Keep app-side all_items intact to avoid transient empty-state windows in the same frame.
        active.all_items = self.all_items.clone();
        active.selected_item = self.selected_item;
        active.selected_file = self.selected_file.clone();
        // PERF: Keep thumbnail when syncing (user might return to this tab)
        active.selected_thumbnail = self.selected_thumbnail.clone();
        active.selected_gif = self.selected_gif.clone();
        active.selected_metadata = self.selected_metadata.clone();
        active.search_query = self.search_query.clone();
        active.scroll_to_selected = self.scroll_to_selected;
        active.scroll_offset_y = self.scroll_offset_y;
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
        active.collapse_local_disks = self.collapse_local_disks;
        active.collapse_network_drives = self.collapse_network_drives;

        // Save per-tab sidebar state (expanded nodes + scroll position)
        active.sidebar_expanded = self.sidebar_tree.snapshot_expanded();
        active.sidebar_scroll_y = self.sidebar_tree.snapshot_scroll_y();

        // On Windows, Path::new(COMPUTER_VIEW_ID).file_name() is None
        if active.is_computer_view {
            active.title = COMPUTER_VIEW_ID.to_string();
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
        let source_tab_id = self.tab_manager.active().id;
        let source_tab_items_len = self.tab_manager.active().items.len();
        let source_tab_all_items_len = self.tab_manager.active().all_items.len();
        let source_tab_selection_len = self.tab_manager.active().multi_selection.len();

        {
            let active = self.tab_manager.active_mut();
            self.navigation_state.current_path = active.path.clone();
            self.navigation_state.path_input = active.path_input.clone();
            self.navigation_state.is_computer_view = active.is_computer_view;
            self.navigation_state.is_recycle_bin_view = active.is_recycle_bin_view;
            self.navigation_state.navigation = active.navigation.clone();
            self.items = active.items.clone();
            self.all_items = std::mem::take(&mut active.all_items);
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
            self.total_items = active.total_items;
            self.view_mode = active.view_mode;
            self.multi_selection = std::mem::take(&mut active.multi_selection);
            self.sort_mode = active.sort_mode;
            self.sort_descending = active.sort_descending;
            self.folders_position = active.folders_position;
            self.show_left_sidebar = active.show_left_sidebar;
            self.show_preview_panel = active.show_preview_panel;
            self.collapse_quick_access = active.collapse_quick_access;
            self.collapse_local_disks = active.collapse_local_disks;
            self.collapse_network_drives = active.collapse_network_drives;
        }

        // Restore per-tab sidebar state (expanded nodes + scroll position)
        {
            let active = self.tab_manager.active();
            let sidebar_expanded = active.sidebar_expanded.clone();
            let sidebar_scroll_y = active.sidebar_scroll_y;
            self.sidebar_tree.restore_expanded(sidebar_expanded, sidebar_scroll_y);
        }

        // Apply folder lock if the destination tab's folder has locked preferences
        self.apply_folder_lock_if_present();

        self.watch_current_folder();

        // If items were cleared (by MoveCompleted event) and this is a regular folder view,
        // trigger a reload to fetch fresh content
        let mut needs_reload = self.items.is_empty()
            && !self.navigation_state.is_computer_view
            && !self.navigation_state.is_recycle_bin_view
            && !self.navigation_state.current_path.is_empty();

        // TAB-SWITCH STALENESS CHECK: Even when the tab has cached items,
        // verify the directory hasn't changed while the tab was inactive.
        // Without this, changes made externally (e.g., in Windows Explorer)
        // won't be visible until the consistency probe catches up (up to 30s).
        if !needs_reload
            && !self.items.is_empty()
            && !self.navigation_state.is_computer_view
            && !self.navigation_state.is_recycle_bin_view
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
            } else if crate::infrastructure::onedrive::is_onedrive_path(&tab_path) {
                false
            } else if self.global_search.available {
                // Try the search service first (NTFS fast path, ~1-2ms via named pipe)
                let path_str = self.navigation_state.current_path.clone();
                match crate::infrastructure::global_search::check_paths_modified(
                    &[path_str],
                    120,
                ) {
                    Ok(modified) => {
                        service_checked = true;
                        !modified.is_empty()
                    }
                    Err(e) => {
                        log::debug!(
                            "[TAB] Search service check_paths_modified failed, falling back to mtime: {}",
                            e
                        );
                        // Fall through to mtime check
                        if let Some(cached_at_ms) = self.directory_cache.cached_at_ms(&tab_path) {
                            let dir_mtime_ms = std::fs::metadata(&tab_path)
                                .ok()
                                .and_then(|m| m.modified().ok())
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_millis() as u64)
                                .unwrap_or(0);
                            dir_mtime_ms > cached_at_ms
                        } else {
                            false
                        }
                    }
                }
            } else if let Some(cached_at_ms) = self.directory_cache.cached_at_ms(&tab_path) {
                // Fallback: compare directory mtime against cache timestamp.
                // Skip for OneDrive paths where metadata calls may block on cloud hydration.
                let dir_mtime_ms = std::fs::metadata(&tab_path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                dir_mtime_ms > cached_at_ms
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
