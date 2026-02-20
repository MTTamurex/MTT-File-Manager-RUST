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
    NotifyIdleVisibleItems(Vec<PathBuf>),
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

    fn notify_idle_visible_items(&mut self, items: Vec<PathBuf>) {
        self.actions.push(ListAction::NotifyIdleVisibleItems(items));
    }

    fn request_icon_load(&mut self, path: PathBuf) {
        self.actions.push(ListAction::RequestIconLoad(path));
    }
}

impl ImageViewerApp {
    /// Render list view with extracted navigation logic
    pub fn render_list_view(&mut self, ui: &mut egui::Ui) {
        // Keyboard navigation (ONLY when not renaming and media is NOT focused)
        if !self.global_search.active
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
                if let Some(selected) = &self.selected_file.clone() {
                    if selected.is_dir {
                        self.navigate_to(&selected.path.to_string_lossy());
                        return; // Exit early after navigation
                    } else {
                        open_with_shell(self, &selected.path);
                    }
                }
            }
        }

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

        // Check if current path is in OneDrive
        // PERFORMANCE: Only use is_onedrive_path() which is string-based (no I/O)
        // path_has_cloud_attributes() was removed because GetFileAttributesW can BLOCK
        // indefinitely on cloud-only OneDrive files, causing UI freeze and crash
        let is_onedrive_folder = {
            let p = PathBuf::from(&self.navigation_state.current_path);
            crate::infrastructure::onedrive::is_onedrive_path(&p)
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

        let mut ctx = ListViewContext {
            items: &items,
            selected_item,
            selected_file: selected_file.as_ref(),
            multi_selection,
            sort_mode,
            sort_descending,
            renaming_state: renaming_state.clone(),
            focus_rename,
            scroll_to_selected,
            is_computer_view: self.navigation_state.is_computer_view,
            is_recycle_bin_view: self.navigation_state.is_recycle_bin_view,
            is_onedrive_folder,
            global_search_active: self.global_search.active,
            texture_cache: &mut self.cache_manager.texture_cache,
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
            col_name_width,
            col_date_width,
            col_type_width,
            col_size_width,
            col_status_width,
        };

        // Use a different approach: collect actions in vectors
        let mut actions = Vec::new();

        let mut ops = ListOps {
            actions: &mut actions,
        };

        let action = list_view::render_list_view(ui, &mut ctx, &mut ops);

        // Update state from context
        self.sort_mode = ctx.sort_mode;
        self.sort_descending = ctx.sort_descending;
        self.renaming_state = ctx.renaming_state;
        // Always consume focus_rename after one frame (cursor selection applied once)
        self.focus_rename = false;

        // Process actions (blocked during renaming)
        let is_renaming = self.renaming_state.is_some();
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
                    self.context_menu.target_paths = selected_paths.clone();

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
            Some(list_view::ListViewAction::EmptyAreaSecondaryClick) if !is_renaming => {
                let path = PathBuf::from(&self.navigation_state.current_path);
                let pointer_pos = ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO);
                let right_bound = ui.available_rect_before_wrap().right();
                self.populate_context_menu(ui.ctx(), std::slice::from_ref(&path), true, None);
                self.context_menu
                    .open(pointer_pos, right_bound, None, vec![path], true);
            }
            _ => {}
        }

        if !is_renaming {
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

                if primary_released {
                    self.complete_item_drag(ctrl, shift);
                }
            }
        } else if self.is_item_dragging {
            self.cancel_item_drag();
        }

        // PERFORMANCE: Collect folder scans for batching (single SQLite query + single filter_items)
        let mut folder_scan_paths: Vec<PathBuf> = Vec::new();

        // Execute collected actions
        for action in actions {
            match action {
                ListAction::NavigateTo(path) => self.navigate_to(&path),
                ListAction::OpenWithShell(path) => open_with_shell(self, &path),
                ListAction::RequestThumbnailLoad(path, size, index, modified) => {
                    self.request_thumbnail_load_with_index_and_modified(path, size, index, modified)
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
                ListAction::NotifyIdleVisibleItems(items) => {
                    let _ = self
                        .file_operation_state
                        .idle_warmup_sender
                        .send(crate::workers::idle_warmup::IdleWarmupMessage::VisibleItems(items));
                }
                ListAction::RequestIconLoad(path) => self.request_icon_load(path),
            }
        }

        // Flush batched folder scans (single SQLite query + single filter_items)
        if !folder_scan_paths.is_empty() {
            self.request_folder_scans_batch(folder_scan_paths);
        }

        // Reset scroll trigger after view has consumed it
        self.scroll_to_selected = false;
    }
}
