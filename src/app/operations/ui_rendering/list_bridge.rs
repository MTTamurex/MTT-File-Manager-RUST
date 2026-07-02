//! List view bridge - connects App state to list_view component
//!
//! This module provides a simplified bridge for rendering the list view,
//! extracting the keyboard navigation and selection logic to shared modules.

use eframe::egui;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::app::operations::navigation::{process_list_keyboard_input, should_handle_navigation};
use crate::app::state::ImageViewerApp;
use crate::infrastructure::io_priority;
use crate::ui::views::rectangle_selection::{RectangleSelectionFrame, RectangleSelectionView};
use crate::ui::views::{list_view, ListViewContext, ListViewOperations};

// Helper function equivalent to open_with_shell from ops
fn open_with_shell(app: &mut ImageViewerApp, path: &Path) {
    app.open_with_shell_guarded(path);
}

/// Action types for list view operations
#[derive(Debug)]
pub enum ListAction {
    NavigateTo(String),
    OpenWithShell(PathBuf),
    RequestThumbnailLoad(PathBuf, u32, usize, u64),
    RequestFolderScan(PathBuf),
    RequestFolderPreviewLoad(PathBuf),
    RenameWithShell(usize),
    RequestThumbnailPrefetchWithIndex(PathBuf, u32, usize, u64),
    RequestIconLoad(PathBuf),
}

/// Operations handler for list view
pub struct ListOps<'a> {
    pub actions: &'a mut Vec<ListAction>,
}

impl<'a> ListViewOperations for ListOps<'a> {
    fn navigate_to(&mut self, path: &str) {
        self.actions.push(ListAction::NavigateTo(path.to_string()));
    }

    fn open_with_shell(&mut self, path: &Path) {
        self.actions
            .push(ListAction::OpenWithShell(path.to_path_buf()));
    }

    fn request_thumbnail_load(&mut self, path: PathBuf, directory_index: usize, modified: u64) {
        // List view always requests small thumbnails (64px)
        self.actions.push(ListAction::RequestThumbnailLoad(
            path,
            64,
            directory_index,
            modified,
        ));
    }

    fn request_thumbnail_load_with_size(
        &mut self,
        path: PathBuf,
        size: u32,
        directory_index: usize,
        modified: u64,
    ) {
        self.actions.push(ListAction::RequestThumbnailLoad(
            path,
            size,
            directory_index,
            modified,
        ));
    }

    fn request_folder_scan(&mut self, path: PathBuf) {
        self.actions.push(ListAction::RequestFolderScan(path));
    }

    fn request_folder_preview_load(&mut self, path: PathBuf) {
        self.actions
            .push(ListAction::RequestFolderPreviewLoad(path));
    }

    fn rename_with_shell(&mut self, idx: usize) {
        self.actions.push(ListAction::RenameWithShell(idx));
    }

    fn request_thumbnail_prefetch_with_index(
        &mut self,
        path: PathBuf,
        size: u32,
        directory_index: usize,
        modified: u64,
    ) {
        self.actions
            .push(ListAction::RequestThumbnailPrefetchWithIndex(
                path,
                size,
                directory_index,
                modified,
            ));
    }

    fn request_icon_load(&mut self, path: PathBuf) {
        self.actions.push(ListAction::RequestIconLoad(path));
    }
}

impl ImageViewerApp {
    /// Render list view with extracted navigation logic
    pub fn render_list_view(&mut self, ui: &mut egui::Ui) {
        let t_total = Instant::now();
        // Keyboard navigation (ONLY when not renaming and media is NOT focused)
        if !self.suppress_file_panel_keyboard
            && !self.global_search.active
            && self.rectangle_selection_state.is_none()
            && should_handle_navigation(
                ui,
                self.renaming_state.is_some(),
                self.is_media_keyboard_focused(),
            )
        {
            let current_index = self.items.iter().position(|x| {
                self.selected_file
                    .as_ref()
                    .is_some_and(|f| f.path == x.path)
            });

            let row_height = 24.0;
            let header_h = 32.0; // Header + Separator precise height for visibility
            let viewport_h = (ui.available_height() - header_h).max(0.0);

            let nav_result = process_list_keyboard_input(
                ui,
                current_index,
                self.items.len(),
                row_height,
                viewport_h,
                Some(
                    self.shortcuts
                        .get(crate::app::shortcuts::ShortcutAction::Properties),
                ),
            );

            let shift = ui.input(|i| i.modifiers.shift);

            // Apply navigation result
            if let Some(new_idx) = nav_result.new_index {
                let clamped = new_idx.min(self.items.len().saturating_sub(1));
                if let Some(item) = self.items.get(clamped) {
                    let item_path = item.path.clone();

                    // UPDATED: Decoupled Focus (selected_item) from Selection (multi_selection)
                    let old_focus = self.selected_item;
                    self.selected_item = Some(clamped);
                    self.selected_file = Some(item.clone());
                    self.update_selected_thumbnail();
                    self.last_keyboard_nav = Instant::now();

                    if shift {
                        // Shift + Arrow/Page: Range selection
                        if self.selection_anchor.is_none() {
                            self.selection_anchor = old_focus;
                        }
                        if let Some(anchor) = self.selection_anchor {
                            let (start, end) = if anchor < clamped {
                                (anchor, clamped)
                            } else {
                                (clamped, anchor)
                            };
                            // Range between anchor and focus (add-only, do NOT clear selection outside range)
                            for i in start..=end {
                                if let Some(it) = self.items.get(i) {
                                    self.multi_selection.insert(it.path.clone());
                                }
                            }
                        }
                    } else {
                        // Navigation without shift: Single-item selection (clear + add focused item)
                        // This ensures the focused item shows the dark blue selection border
                        self.multi_selection.clear();
                        self.multi_selection.insert(item_path.clone());
                        self.selection_anchor = Some(clamped);
                    }

                    // Trigger scroll normalization in the view
                    self.scroll_to_selected = true;

                    // Request visibility for the new selected index
                    self.scroll_request = crate::app::state::ScrollRequest::EnsureVisible(clamped);

                    ui.ctx().request_repaint();
                }
            }

            // Enter to open (only when not renaming)
            if nav_result.enter_pressed {
                if self.suppress_next_enter_open {
                    self.suppress_next_enter_open = false;
                } else if let Some(selected) = self.selected_file.as_ref() {
                    let selected_path = selected.path.clone();
                    if selected.is_dir {
                        let target = selected_path.to_string_lossy();
                        self.navigate_to(target.as_ref());
                        return; // Exit early after navigation
                    } else {
                        open_with_shell(self, &selected_path);
                    }
                }
            } else if self.suppress_next_enter_open {
                self.suppress_next_enter_open = false;
            }
        }

        let t_after_nav = Instant::now();

        // Extract data to avoid multiple borrows
        let items = self.items.clone();
        let selected_item = self.selected_item;
        let selected_file = self.selected_file.clone();
        let sort_mode = self.sort_mode;
        let sort_descending = self.sort_descending;
        let renaming_state = self.renaming_state.clone();
        let focus_rename = self.focus_rename;
        let folder_icon_texture = self.cache_manager.folder_icon_texture.clone();
        let computer_icon = self.cache_manager.computer_icon.clone();

        // Check if current path is in a Cloud Files sync root.
        // PERFORMANCE: Only use is_cloud_sync_path() which is string-based (no I/O)
        // path_has_cloud_attributes() was removed because GetFileAttributesW can BLOCK
        // indefinitely on cloud-only provider files, causing UI freeze and crash
        let is_onedrive_folder = {
            let p = PathBuf::from(&self.navigation_state.current_path);
            crate::infrastructure::onedrive::is_cloud_sync_path(&p)
        };

        // Create context with separate mutable references
        let scroll_to_selected = self.scroll_to_selected;
        let is_video_docked_visible = self.is_video_docked_visible();
        let multi_selection = &self.multi_selection;
        // Non-blocking in render loop: use cached profile only.
        // Unknown drives fall back to HDD behavior to avoid UI stalls.
        let is_ssd = io_priority::try_is_ssd(Path::new(&self.navigation_state.current_path))
            .unwrap_or(false);
        let prefetch_rows = if is_ssd { 1 } else { 3 };
        let mut drag_started_item = None;
        let mut drag_hovered_item = None;
        let mut rectangle_selection_frame = RectangleSelectionFrame::default();
        let rectangle_selection_state = self
            .rectangle_selection_state
            .as_ref()
            .filter(|state| state.view == RectangleSelectionView::List);

        // Select appropriate column width references based on context
        let (col_name_width, col_date_width, col_type_width, col_size_width, col_status_width) =
            if self.navigation_state.is_computer_view {
                // Computer view uses its own set of columns
                (
                    &mut self.layout.list_col_computer_name_width,
                    &mut self.layout.list_col_computer_total_width,
                    &mut self.layout.list_col_type_width, // Not used in computer view
                    &mut self.layout.list_col_computer_free_width,
                    &mut self.layout.list_col_onedrive_status_width, // Not used in computer view
                )
            } else if is_onedrive_folder {
                // OneDrive view uses its own set with status column
                (
                    &mut self.layout.list_col_onedrive_name_width,
                    &mut self.layout.list_col_onedrive_date_width,
                    &mut self.layout.list_col_onedrive_type_width,
                    &mut self.layout.list_col_onedrive_size_width,
                    &mut self.layout.list_col_onedrive_status_width,
                )
            } else {
                // Regular view uses standard columns
                (
                    &mut self.layout.list_col_name_width,
                    &mut self.layout.list_col_date_width,
                    &mut self.layout.list_col_type_width,
                    &mut self.layout.list_col_size_width,
                    &mut self.layout.list_col_onedrive_status_width, // Not used in regular view
                )
            };

        // Auto-fit list columns when transitioning from dual-panel to mono-panel.
        // Measures content for Size/Type/Date columns and gives remaining space to Name.
        // Save is deferred until after ListViewContext is dropped (borrow release).
        // If items are empty, keep the flag so auto-fit retries on the next render
        // with actual content.
        let needs_save_after_autofit = self.pending_list_column_autofit && !items.is_empty();
        if self.pending_list_column_autofit && !items.is_empty() {
            self.pending_list_column_autofit = false;
            list_view::auto_fit_columns(
                ui,
                &items,
                self.navigation_state.is_computer_view,
                self.navigation_state.is_recycle_bin_view,
                is_onedrive_folder,
                ui.available_width(),
                col_name_width,
                col_date_width,
                col_type_width,
                col_size_width,
                col_status_width,
                &self.folder_size_state.batch_cache,
            );
        }

        let mut folder_size_requests: Vec<PathBuf> = Vec::new();

        let mut ctx = ListViewContext {
            items: &items,
            selected_item,
            selected_file: selected_file.as_ref(),
            multi_selection,
            sort_mode,
            sort_descending,
            renaming_state,
            focus_rename,
            scroll_to_selected,
            generation: self.generation,
            is_computer_view: self.navigation_state.is_computer_view,
            is_recycle_bin_view: self.navigation_state.is_recycle_bin_view,
            is_onedrive_folder,
            global_search_active: self.global_search.active,
            texture_cache: &mut self.cache_manager.texture_cache,
            attempted_thumbnail_bucket: &self.cache_manager.attempted_thumbnail_bucket,
            loading_set: &mut self.cache_manager.loading_set,
            loading_icons: &mut self.loading_icons,
            failed_icons: &self.failed_icons,
            scanned_folders: &mut self.scanned_folders,
            folder_icon_texture: folder_icon_texture.as_ref(),
            computer_icon: computer_icon.as_ref(),
            drive_icon_cache: &mut self.cache_manager.drive_icon_cache,
            item_icon_loader: &mut self.item_icon_loader,
            deletion_date_cache: Some(&mut self.deletion_date_cache),
            failed_thumbnails: &self.cache_manager.failed_thumbnails,
            scroll_offset_y: self.scroll_offset_y,
            mut_scroll_offset_y: &mut self.scroll_offset_y,
            last_input: self.last_input,
            last_scroll_time: &mut self.last_scroll_time,
            last_scroll_offset: &mut self.last_scroll_offset,
            pending_upload_set: &mut self.cache_manager.pending_upload_set,
            is_video_docked_visible,
            is_on_hdd: !is_ssd,
            prefetch_rows,
            visible_index_range: &mut self.visible_index_range,
            is_item_dragging: self.is_item_dragging,
            drag_target_folder: self.drag_target_folder.clone(),
            drag_started_item: &mut drag_started_item,
            drag_hovered_item: &mut drag_hovered_item,
            rectangle_selection_state,
            rectangle_selection_frame: &mut rectangle_selection_frame,
            live_file_size_cache: &mut self.live_file_size_cache,
            live_file_size_loading: &mut self.live_file_size_loading,
            live_file_size_req_sender: &self.live_file_size_req_sender,
            show_preview_panel: self.show_preview_panel,
            thumbnail_requests_this_frame: 0,
            folder_size_cache: &self.folder_size_state.batch_cache,
            folder_size_batch_loading: &self.folder_size_state.batch_loading,
            folder_size_requests: &mut folder_size_requests,
            col_name_width,
            col_date_width,
            col_type_width,
            col_size_width,
            col_status_width,
            tag_assignments: self.tag_assignments_normalized.as_ref(),
            tag_definitions: &self.tag_definitions,
        };

        // Use a different approach: collect actions in vectors
        let mut actions = Vec::new();

        let mut ops = ListOps {
            actions: &mut actions,
        };

        let t_after_prepare = Instant::now();

        let action = list_view::render_list_view(ui, &mut ctx, &mut ops);

        let t_after_core_render = Instant::now();

        // Extract values from context before dropping it (releases borrows on self).
        let sort_mode = ctx.sort_mode;
        let sort_descending = ctx.sort_descending;
        let renaming_state = ctx.renaming_state.take();
        drop(ctx);

        // Update state
        self.sort_mode = sort_mode;
        self.sort_descending = sort_descending;
        self.renaming_state = renaming_state;
        // Always consume focus_rename after one frame (cursor selection applied once)
        self.focus_rename = false;

        // Persist auto-fitted column widths (deferred from before ctx creation).
        if needs_save_after_autofit {
            self.save_preferences();
        }

        let file_panel_input_blocked = self.file_panel_input_blocked_by_drag_move_confirmation();
        if file_panel_input_blocked {
            self.cancel_rectangle_selection();
        } else {
            let suppress_rectangle_start = drag_started_item.is_some();
            self.handle_rectangle_selection_frame(
                ui,
                &rectangle_selection_frame,
                suppress_rectangle_start,
            );
        }

        // ── Send batch folder-size requests (capped per frame) ──
        {
            const MAX_BATCH_REQUESTS_PER_FRAME: usize = 30;
            let gen = self
                .folder_size_state
                .batch_generation
                .load(std::sync::atomic::Ordering::Acquire);
            for path in folder_size_requests
                .into_iter()
                .take(MAX_BATCH_REQUESTS_PER_FRAME)
            {
                let epoch = self
                    .folder_size_state
                    .batch_invalidation_epoch
                    .get(&path)
                    .copied()
                    .unwrap_or(0);
                self.folder_size_state.batch_loading.insert(path.clone());
                let _ = self
                    .folder_size_state
                    .batch_req_sender
                    .send((path, gen, epoch));
            }
        }

        // Process actions (blocked during renaming)
        let is_renaming = self.renaming_state.is_some();
        if !file_panel_input_blocked {
            match action {
                Some(list_view::ListViewAction::Click(idx)) if !is_renaming => {
                    if let Some(item) = self.items.get(idx) {
                        let ctrl = ui.input(|i| i.modifiers.ctrl);
                        let shift = ui.input(|i| i.modifiers.shift);

                        if ctrl {
                            // Ctrl + Click: Toggle item and set focus/anchor
                            if self.multi_selection.contains(&item.path) {
                                self.multi_selection.remove(&item.path);
                            } else {
                                self.multi_selection.insert(item.path.clone());
                            }
                            self.selected_item = Some(idx);
                            self.selection_anchor = Some(idx);
                            self.selected_file = Some(item.clone());
                        } else if shift {
                            // Shift + Click: Range between anchor and click
                            if let Some(anchor) = self.selection_anchor {
                                let (start, end) = if anchor < idx {
                                    (anchor, idx)
                                } else {
                                    (idx, anchor)
                                };
                                // Add range to selection (Do NOT clear outside selection as requested)
                                for i in start..=end {
                                    if let Some(it) = self.items.get(i) {
                                        self.multi_selection.insert(it.path.clone());
                                    }
                                }
                                self.selected_item = Some(idx);
                                self.selected_file = Some(item.clone());
                            } else {
                                // Fallback: simple insert
                                self.multi_selection.insert(item.path.clone());
                                self.selected_item = Some(idx);
                                self.selection_anchor = Some(idx);
                                self.selected_file = Some(item.clone());
                            }
                        } else {
                            // Simple Click: Reset selection to target and set focus/anchor
                            self.multi_selection.clear();
                            self.multi_selection.insert(item.path.clone());
                            self.selected_item = Some(idx);
                            self.selection_anchor = Some(idx);
                            self.selected_file = Some(item.clone());
                        }

                        // Common updates
                        self.update_selected_thumbnail();
                        ui.ctx().request_repaint();
                    }
                }
                Some(list_view::ListViewAction::DoubleClick(idx)) if !is_renaming => {
                    let mut path_to_navigate = None;
                    if let Some(item) = self.items.get(idx) {
                        if item.is_dir {
                            if !self.navigation_state.is_recycle_bin_view {
                                path_to_navigate = Some(item.path.clone());
                            }
                        } else {
                            let path = item.path.clone();
                            let extension = path
                                .extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("")
                                .to_lowercase();
                            if extension == "iso" {
                                self.mount_and_navigate_iso(path);
                            } else {
                                open_with_shell(self, &path);
                            }
                        }
                    }

                    if let Some(path) = path_to_navigate {
                        self.navigate_to(&path.to_string_lossy());
                    }
                }
                Some(list_view::ListViewAction::SecondaryClick(idx)) if !is_renaming => {
                    if let Some(item) = self.items.get(idx) {
                        // Update selection logic for right-click
                        if !self.multi_selection.contains(&item.path) {
                            self.multi_selection.clear();
                            self.multi_selection.insert(item.path.clone());
                            self.selected_item = Some(idx);
                            self.selected_file = Some(item.clone());
                        } else {
                            self.selected_item = Some(idx);
                            self.selected_file = Some(item.clone());
                        }

                        // Collect all selected paths
                        let selected_paths: Vec<PathBuf> =
                            self.multi_selection.iter().cloned().collect();

                        // Use the new styled menu system
                        let pointer_pos = ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO);
                        let right_bound = ui.available_rect_before_wrap().right();

                        // Populate with multiple paths
                        self.populate_context_menu(ui.ctx(), &selected_paths, false, Some(idx));
                        self.context_menu.open(
                            pointer_pos,
                            right_bound,
                            Some(idx),
                            selected_paths,
                            false,
                        );
                    }
                }
                Some(list_view::ListViewAction::SortChange(mode)) => {
                    // Toggle direction if same mode, otherwise switch mode
                    if self.sort_mode == mode {
                        self.sort_descending = !self.sort_descending;
                    } else {
                        self.sort_mode = mode;
                        self.sort_descending = false;
                    }
                    if !self.current_folder_locked {
                        self.sort_mode_normal = self.sort_mode;
                        self.sort_descending_normal = self.sort_descending;
                    }
                    self.sort_items();
                    self.save_preferences();
                }
                Some(list_view::ListViewAction::EmptyAreaSecondaryClick)
                    if !is_renaming && self.can_open_empty_area_context_menu() =>
                {
                    let path = PathBuf::from(&self.navigation_state.current_path);
                    let pointer_pos = ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO);
                    let right_bound = ui.available_rect_before_wrap().right();
                    self.populate_context_menu(ui.ctx(), std::slice::from_ref(&path), true, None);
                    self.context_menu
                        .open(pointer_pos, right_bound, None, vec![path], true);
                }
                Some(list_view::ListViewAction::EmptyAreaClick) if !is_renaming => {
                    self.clear_file_view_selection();
                }
                _ => {}
            }
        }

        self.warm_detail_panel_folder_preview();

        if !file_panel_input_blocked && !is_renaming && self.rectangle_selection_state.is_none() {
            if let Some(start_idx) = drag_started_item {
                self.begin_item_drag(start_idx);
            }

            if self.is_item_dragging {
                self.update_item_drag_target_from_hover(drag_hovered_item);
                // Cursor feedback and drag ghost are rendered in app_impl.rs
                // after all UI, so no widget can override the cursor.
                let (ctrl, shift, primary_released) = ui.input(|i| {
                    (
                        i.modifiers.ctrl,
                        i.modifiers.shift,
                        i.pointer.primary_released(),
                    )
                });

                // When the mouse is over the inactive panel (cross-panel drag),
                // defer to the inactive panel's bridge so drag_target_folder
                // is resolved from the inactive panel's items (subfolder support).
                if primary_released
                    && (self.drag_cross_panel_target.is_none()
                        || self.drag_drop_cross_panel_context)
                {
                    self.complete_item_drag(ctrl, shift);
                }
            }
        } else if self.is_item_dragging {
            self.cancel_item_drag();
        }

        let t_after_interactions = Instant::now();

        // PERFORMANCE: Collect folder scans for batching (single SQLite query + single filter_items)
        let mut folder_scan_paths: Vec<PathBuf> = Vec::new();

        let selected_path_for_thumbnail_priority =
            self.selected_file.as_ref().map(|f| f.path.clone());

        // Execute collected actions
        for action in actions {
            match action {
                ListAction::NavigateTo(path) => self.navigate_to(&path),
                ListAction::OpenWithShell(path) => open_with_shell(self, &path),
                ListAction::RequestThumbnailLoad(path, size, index, modified) => {
                    let directory_index =
                        if selected_path_for_thumbnail_priority.as_ref() == Some(&path) {
                            0
                        } else {
                            index.saturating_add(1)
                        };
                    self.request_thumbnail_load_with_index_and_modified(
                        path,
                        size,
                        directory_index,
                        modified,
                    );
                }
                ListAction::RequestFolderScan(path) => folder_scan_paths.push(path),
                ListAction::RequestFolderPreviewLoad(path) => {
                    self.request_folder_preview_load(path)
                }
                ListAction::RenameWithShell(idx) => self.rename_with_shell(idx),
                ListAction::RequestThumbnailPrefetchWithIndex(path, size, index, modified) => self
                    .request_thumbnail_prefetch_with_index_and_modified(
                        path, size, index, modified,
                    ),
                ListAction::RequestIconLoad(path) => self.request_icon_load(path),
            }
        }

        // Flush batched folder scans (single SQLite query + single filter_items)
        if !folder_scan_paths.is_empty() {
            self.request_folder_scans_batch(folder_scan_paths);
        }

        // Reset scroll trigger after view has consumed it
        self.scroll_to_selected = false;

        let total_ms = t_total.elapsed().as_millis();
        if total_ms > 120 {
            log::warn!(
                "[PERF-CENTRAL-LIST] total={}ms nav={}ms prepare={}ms core_render={}ms interactions={}ms exec_actions={}ms items={} visible={:?} loading_icons={} pending_uploads={}",
                total_ms,
                t_after_nav.duration_since(t_total).as_millis(),
                t_after_prepare.duration_since(t_after_nav).as_millis(),
                t_after_core_render.duration_since(t_after_prepare).as_millis(),
                t_after_interactions.duration_since(t_after_core_render).as_millis(),
                t_total.elapsed().as_millis().saturating_sub(t_after_interactions.duration_since(t_total).as_millis()),
                self.items.len(),
                self.visible_index_range,
                self.loading_icons.len(),
                self.cache_manager.pending_upload_set.len(),
            );
        }
    }
}
