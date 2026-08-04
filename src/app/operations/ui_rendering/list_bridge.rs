//! List view bridge - connects App state to list_view component
//!
//! This module provides a simplified bridge for rendering the list view,
//! extracting the keyboard navigation and selection logic to shared modules.

use eframe::egui;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::app::operations::navigation::{
    process_column_list_keyboard_input, process_list_keyboard_input, should_handle_navigation,
};
use crate::app::state::ImageViewerApp;
use crate::infrastructure::io_priority;
use crate::ui::views::rectangle_selection::{
    RectangleSelectionFrame, RectangleSelectionSource, RectangleSelectionView,
};
use crate::ui::views::{list_view, ListViewContext, ListViewOperations};

// Helper function equivalent to open_with_shell from ops
fn open_with_shell(app: &mut ImageViewerApp, path: &Path) {
    app.open_with_shell_guarded(path);
}

fn should_auto_open_compact_folder(
    compact: bool,
    is_directory: bool,
    ctrl: bool,
    shift: bool,
    is_recycle_bin: bool,
) -> bool {
    compact && is_directory && !ctrl && !shift && !is_recycle_bin
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
        self.render_list_like_view(ui, false, false);
    }

    pub(crate) fn render_column_list_bridge(&mut self, ui: &mut egui::Ui) {
        self.render_list_like_view(ui, true, false);
    }

    /// Render the details list view in compact (name-only) mode — no header,
    /// no Date/Type/Size columns. Used by the Miller's Columns focused column
    /// so it keeps the full interaction stack while looking like a column.
    pub(crate) fn render_list_view_compact(&mut self, ui: &mut egui::Ui) {
        self.render_list_like_view(ui, false, true);
    }

    fn render_list_like_view(&mut self, ui: &mut egui::Ui, column_list: bool, compact: bool) {
        let t_total = Instant::now();
        let collapsed_groups = self
            .collapsed_groups_by_context
            .get(&self.grouping_context_key())
            .cloned()
            .unwrap_or_default();
        // Keyboard navigation (ONLY when not renaming and media is NOT focused)
        if !self.suppress_file_panel_keyboard
            && !self.global_search.active
            && self.rectangle_selection_state.is_none()
            && ui.input(|input| {
                [
                    egui::Key::ArrowUp,
                    egui::Key::ArrowDown,
                    egui::Key::ArrowLeft,
                    egui::Key::ArrowRight,
                    egui::Key::PageUp,
                    egui::Key::PageDown,
                    egui::Key::Enter,
                ]
                .into_iter()
                .any(|key| input.key_pressed(key))
            })
            && should_handle_navigation(
                ui,
                self.renaming_state.is_some(),
                self.is_media_keyboard_focused(),
            )
        {
            let visible_indices = if compact {
                (0..self.items.len()).collect::<Vec<_>>()
            } else {
                self.visible_group_item_indices()
            };
            let mut visual_slots: Vec<Option<usize>> =
                visible_indices.iter().copied().map(Some).collect();
            let mut navigation_block_size = 1usize;
            let rows = if column_list {
                let group_counts: Vec<usize> = self
                    .group_projection
                    .sections
                    .iter()
                    .map(|section| {
                        if collapsed_groups.contains(&section.key) {
                            1
                        } else {
                            section.item_indices.len().max(1)
                        }
                    })
                    .collect();
                let rows = if self.group_projection.is_grouped() {
                    crate::ui::views::column_list_view::column_list_grouped_rows_for_counts(
                        &group_counts,
                        ui.available_width(),
                        ui.available_height(),
                    )
                } else {
                    crate::ui::views::column_list_view::column_list_rows(
                        self.items.len(),
                        ui.available_width(),
                        ui.available_height(),
                    )
                };
                visual_slots = crate::application::grouping::column_visual_slots(
                    &self.group_projection,
                    Some(&collapsed_groups),
                    self.items.len(),
                    rows,
                );
                navigation_block_size = rows;
                Some(rows)
            } else {
                None
            };
            let current_index = self.selected_item.and_then(|selected| {
                visual_slots
                    .iter()
                    .position(|index| *index == Some(selected))
            });

            let reserved_enter = Some(
                self.shortcuts
                    .get(crate::app::shortcuts::ShortcutAction::Properties),
            );
            let nav_result = if column_list {
                let visible_columns =
                    crate::ui::views::column_list_view::column_list_visible_columns(
                        ui.available_width(),
                    );
                process_column_list_keyboard_input(
                    ui,
                    current_index,
                    visual_slots.len(),
                    rows.unwrap_or(1),
                    visible_columns,
                    reserved_enter,
                )
            } else {
                let row_height = 24.0;
                let header_h = if compact { 0.0 } else { 32.0 };
                let viewport_h = (ui.available_height() - header_h).max(0.0);
                let navigation_viewport_h = if !compact && self.group_projection.is_grouped() {
                    grouped_visible_list_item_count(
                        &self.group_projection,
                        &collapsed_groups,
                        self.scroll_offset_y,
                        viewport_h,
                        row_height,
                    ) as f32
                        * row_height
                } else {
                    viewport_h
                };
                process_list_keyboard_input(
                    ui,
                    current_index,
                    visual_slots.len(),
                    row_height,
                    navigation_viewport_h.max(row_height),
                    reserved_enter,
                )
            };

            let shift = ui.input(|i| i.modifiers.shift);
            let selected_is_visible = self
                .selected_item
                .is_some_and(|selected| visual_slots.contains(&Some(selected)));

            // Apply navigation result
            if let Some(new_idx) = nav_result.new_index {
                if let Some(clamped) = crate::application::grouping::resolve_visual_slot(
                    &visual_slots,
                    new_idx,
                    current_index,
                    navigation_block_size,
                    false,
                ) {
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
                                let range = if compact {
                                    let (start, end) = if anchor <= clamped {
                                        (anchor, clamped)
                                    } else {
                                        (clamped, anchor)
                                    };
                                    (start..=end).collect()
                                } else {
                                    self.visible_group_range(anchor, clamped)
                                };
                                for i in range {
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
                        self.scroll_request =
                            crate::app::state::ScrollRequest::EnsureVisible(clamped);

                        ui.ctx().request_repaint();
                    }
                }
            }

            // Enter to open (only when not renaming)
            if nav_result.enter_pressed && selected_is_visible {
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
        let group_projection = self.group_projection.clone();
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
        let multi_selection = &self.multi_selection;
        // Non-blocking in render loop: use cached profile only.
        // Unknown drives fall back to HDD behavior to avoid UI stalls.
        let is_ssd = io_priority::try_is_ssd(Path::new(&self.navigation_state.current_path))
            .unwrap_or(false);
        let prefetch_rows = if is_ssd { 1 } else { 3 };
        let mut drag_started_item = None;
        let mut drag_hovered_item = None;
        let mut rectangle_selection_frame = RectangleSelectionFrame::default();
        let external_drop_over_this_panel = self.external_drop_active
            && ui
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|pos| ui.clip_rect().contains(pos));
        let rectangle_selection_state = self.rectangle_selection_state.as_ref().filter(|state| {
            state.source == RectangleSelectionSource::CurrentItems
                && state.view
                    == if column_list {
                        RectangleSelectionView::ColumnList
                    } else {
                        RectangleSelectionView::List
                    }
        });

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
        let needs_save_after_autofit =
            !column_list && !compact && self.pending_list_column_autofit && !items.is_empty();
        if !column_list && !compact && self.pending_list_column_autofit && !items.is_empty() {
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
            group_projection: &group_projection,
            collapsed_groups: &collapsed_groups,
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
            mut_scroll_offset_x: &mut self.scroll_offset_x,
            last_input: self.last_input,
            last_scroll_time: &mut self.last_scroll_time,
            last_scroll_offset: &mut self.last_scroll_offset,
            pending_upload_set: &mut self.cache_manager.pending_upload_set,
            is_on_hdd: !is_ssd,
            prefetch_rows,
            visible_index_range: &mut self.visible_index_range,
            visible_group_paths: &mut self.visible_group_paths,
            is_item_dragging: self.is_item_dragging || external_drop_over_this_panel,
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
            compact,
        };

        // Use a different approach: collect actions in vectors
        let mut actions = Vec::new();

        let mut ops = ListOps {
            actions: &mut actions,
        };

        let t_after_prepare = Instant::now();

        let action = if column_list {
            crate::ui::views::column_list_view::render_column_list_view(ui, &mut ctx, &mut ops)
        } else {
            list_view::render_list_view(ui, &mut ctx, &mut ops)
        };

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

        if matches!(
            action.as_ref(),
            Some(
                list_view::ListViewAction::SecondaryClick(_)
                    | list_view::ListViewAction::EmptyAreaSecondaryClick
            )
        ) {
            self.finish_rectangle_selection();
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
                    let (ctrl, shift) =
                        ui.input(|input| (input.modifiers.ctrl, input.modifiers.shift));
                    let auto_open_path = self.items.get(idx).and_then(|item| {
                        should_auto_open_compact_folder(
                            compact,
                            item.is_dir,
                            ctrl,
                            shift,
                            self.navigation_state.is_recycle_bin_view,
                        )
                        .then(|| item.path.clone())
                    });
                    if let Some(path) = auto_open_path {
                        self.navigate_to(&path.to_string_lossy());
                        return;
                    }

                    if let Some(item) = self.items.get(idx) {
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
                                let range = if compact {
                                    let (start, end) = if anchor <= idx {
                                        (anchor, idx)
                                    } else {
                                        (idx, anchor)
                                    };
                                    (start..=end).collect()
                                } else {
                                    self.visible_group_range(anchor, idx)
                                };
                                for i in range {
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
                        let selected_paths =
                            super::ordered_context_menu_paths(&item.path, &self.multi_selection);
                        let primary_is_directory = item.is_dir || item.drive_info.is_some();
                        let operation_directory =
                            PathBuf::from(&self.navigation_state.current_path);

                        // Use the new styled menu system
                        let pointer_pos = ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO);
                        // Populate with multiple paths
                        self.populate_context_menu(ui.ctx(), &selected_paths, false, Some(idx));
                        self.context_menu
                            .open(pointer_pos, Some(idx), selected_paths, false);
                        self.context_menu.primary_is_directory = Some(primary_is_directory);
                        self.context_menu.operation_directory = Some(operation_directory);
                        self.capture_context_menu_panel_origin();
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
                Some(list_view::ListViewAction::ToggleGroup(key)) if !is_renaming => {
                    self.toggle_group_collapsed(key);
                    ui.ctx().request_repaint();
                }
                Some(list_view::ListViewAction::EmptyAreaSecondaryClick)
                    if !is_renaming && self.can_open_empty_area_context_menu() =>
                {
                    let path = PathBuf::from(&self.navigation_state.current_path);
                    let pointer_pos = ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO);
                    self.populate_context_menu(ui.ctx(), std::slice::from_ref(&path), true, None);
                    self.context_menu.open(pointer_pos, None, vec![path], true);
                    self.capture_context_menu_panel_origin();
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
            } else if external_drop_over_this_panel {
                self.update_external_drop_hover(drag_hovered_item);
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
                ListAction::RequestIconLoad(path) => {
                    let _ =
                        self.request_icon_load(path, crate::domain::file_entry::IconSize::Large);
                }
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

fn grouped_visible_list_item_count(
    projection: &crate::application::grouping::GroupProjection,
    collapsed: &rustc_hash::FxHashSet<crate::application::grouping::GroupKey>,
    scroll_y: f32,
    viewport_height: f32,
    row_height: f32,
) -> usize {
    let viewport_bottom = scroll_y + viewport_height;
    let mut content_y = 0.0;
    let mut count = 0usize;
    for section in &projection.sections {
        content_y += crate::ui::views::group_header::GROUP_HEADER_HEIGHT;
        if !collapsed.contains(&section.key) {
            let items_top = content_y;
            let items_bottom = items_top + section.item_indices.len() as f32 * row_height;
            if items_bottom > scroll_y && items_top < viewport_bottom {
                let first = ((scroll_y - items_top) / row_height).floor().max(0.0) as usize;
                let last = ((viewport_bottom - items_top) / row_height).ceil().max(0.0) as usize;
                count += last.min(section.item_indices.len()).saturating_sub(first);
            }
            content_y = items_bottom;
        }
        content_y += crate::ui::views::group_header::GROUP_GAP;
    }
    count.max(1)
}

#[cfg(test)]
mod tests {
    use super::should_auto_open_compact_folder;

    #[test]
    fn compact_folder_auto_open_requires_an_unmodified_click() {
        assert!(should_auto_open_compact_folder(
            true, true, false, false, false
        ));
        assert!(!should_auto_open_compact_folder(
            true, true, true, false, false
        ));
        assert!(!should_auto_open_compact_folder(
            true, true, false, true, false
        ));
        assert!(!should_auto_open_compact_folder(
            false, true, false, false, false
        ));
        assert!(!should_auto_open_compact_folder(
            true, false, false, false, false
        ));
        assert!(!should_auto_open_compact_folder(
            true, true, false, false, true
        ));
    }
}
