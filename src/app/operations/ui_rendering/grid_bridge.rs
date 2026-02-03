//! Grid view bridge - connects App state to grid_view component
//!
//! This module provides a simplified bridge for rendering the grid view,
//! extracting the keyboard navigation logic to shared modules.

use eframe::egui;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::app::operations::navigation::{process_grid_keyboard_input, should_handle_navigation};
use crate::app::state::ImageViewerApp;
use crate::infrastructure::io_priority;
use crate::ui::views::{grid_view, GridViewContext, GridViewOperations};

// Helper function equivalent to open_with_shell from ops
fn open_with_shell(path: &Path) {
    let _ = crate::application::file_operations::open_with_shell(path, None);
}

/// Action types for grid view operations
#[derive(Debug)]
pub enum GridAction {
    NavigateTo(String),
    OpenWithShell(PathBuf),
    RequestThumbnailLoad(PathBuf, u32, u64),
    RequestThumbnailLoadWithIndex(PathBuf, u32, usize, u64),
    RequestFolderScan(PathBuf),
    RequestFolderPreviewLoad(PathBuf),
    RequestThumbnailPrefetch(PathBuf, u32, u64),
    RequestThumbnailPrefetchWithIndex(PathBuf, u32, usize, u64),
    RequestIconLoad(PathBuf),
    RenameWithShell(usize),
    NotifyIdleVisibleItems(Vec<PathBuf>),
}

/// Operations handler for grid view
pub struct GridOps<'a> {
    pub actions: &'a mut Vec<GridAction>,
}

impl<'a> GridViewOperations for GridOps<'a> {
    fn navigate_to(&mut self, path: &str) {
        self.actions.push(GridAction::NavigateTo(path.to_string()));
    }

    fn open_with_shell(&mut self, path: &PathBuf) {
        self.actions.push(GridAction::OpenWithShell(path.clone()));
    }

    fn request_thumbnail_load(&mut self, path: PathBuf, size: u32, modified: u64) {
        self.actions
            .push(GridAction::RequestThumbnailLoad(path, size, modified));
    }

    fn request_thumbnail_load_with_index(
        &mut self,
        path: PathBuf,
        size: u32,
        directory_index: usize,
        modified: u64,
    ) {
        self.actions.push(GridAction::RequestThumbnailLoadWithIndex(
            path,
            size,
            directory_index,
            modified,
        ));
    }

    fn request_folder_scan(&mut self, path: PathBuf) {
        self.actions.push(GridAction::RequestFolderScan(path));
    }
    fn request_folder_preview_load(&mut self, path: PathBuf) {
        self.actions
            .push(GridAction::RequestFolderPreviewLoad(path));
    }

    fn request_thumbnail_prefetch(&mut self, path: PathBuf, size: u32, modified: u64) {
        self.actions
            .push(GridAction::RequestThumbnailPrefetch(path, size, modified));
    }

    fn request_thumbnail_prefetch_with_index(
        &mut self,
        path: PathBuf,
        size: u32,
        directory_index: usize,
        modified: u64,
    ) {
        self.actions
            .push(GridAction::RequestThumbnailPrefetchWithIndex(
                path,
                size,
                directory_index,
                modified,
            ));
    }

    fn request_icon_load(&mut self, path: PathBuf) {
        self.actions.push(GridAction::RequestIconLoad(path));
    }

    fn rename_with_shell(&mut self, idx: usize) {
        self.actions.push(GridAction::RenameWithShell(idx));
    }

    fn notify_idle_visible_items(&mut self, items: Vec<PathBuf>) {
        self.actions.push(GridAction::NotifyIdleVisibleItems(items));
    }
}

impl ImageViewerApp {
    /// Render grid view with extracted navigation logic
    pub fn render_grid_view(&mut self, ui: &mut egui::Ui) {
        // Calculate cols for keyboard navigation
        let padding = 8.0;
        let item_w = self.thumbnail_size;
        let item_h = self.thumbnail_size + 20.0;
        let cell_h = item_h + padding;
        let available_w = ui.available_width();
        let cols = ((available_w - padding) / (item_w + padding))
            .floor()
            .max(1.0) as usize;

        // Keyboard navigation (ONLY when not renaming and media is NOT focused)
        if should_handle_navigation(ui, self.renaming_state.is_some(), self.is_media_keyboard_focused()) {
            let current_index = self.items.iter().position(|x| {
                self.selected_file
                    .as_ref()
                    .map_or(false, |f| f.path == x.path)
            });

            let nav_result = process_grid_keyboard_input(
                ui,
                current_index,
                self.items.len(),
                cols,
                cell_h,
                ui.available_height(),
            );

            let shift = ui.input(|i| i.modifiers.shift);

            if let Some(new_idx) = nav_result.new_index {
                let clamped = new_idx.min(self.items.len().saturating_sub(1)) as usize;
                if let Some(item) = self.items.get(clamped) {
                    // Clone path before any mutable borrows
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
                            // Add range between anchor and focus (NÃO limpar seleção fora do range)
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
                        self.multi_selection.insert(item_path);
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
                        open_with_shell(&selected.path);
                    }
                }
            }
        }

        // Extract data to avoid multiple borrows
        let items = self.items.clone();
        let selected_item = self.selected_item;
        let selected_file = self.selected_file.clone();
        let thumbnail_size = self.thumbnail_size;
        let last_grid_cols = self.last_grid_cols;
        let renaming_state = self.renaming_state.clone();
        let focus_rename = self.focus_rename;
        let folder_icon_texture = self.cache_manager.folder_icon_texture.clone();
        let computer_icon = self.cache_manager.computer_icon.clone();

        // Create context with separate mutable references
        let scroll_to_selected = self.scroll_to_selected;
        let multi_selection = &self.multi_selection;

        // PERFORMANCE: Clear shared buffers before rendering (reuse, don't reallocate)
        self.pending_ops.clear();

        // Check if video is playing in docked mode to reduce disk I/O
        let is_video_docked_visible = self.is_video_docked_visible();

        let is_ssd = io_priority::is_ssd(&PathBuf::from(&self.current_path));
        let prefetch_rows = if is_ssd { 1 } else { 3 };
        let mut ctx = GridViewContext {
            items: &items,
            selected_item,
            selected_file: selected_file.as_ref(),
            multi_selection,
            thumbnail_size,
            last_grid_cols,
            renaming_state: renaming_state.clone(),
            focus_rename,
            scroll_to_selected,
            is_computer_view: self.is_computer_view,
            is_recycle_bin_view: self.is_recycle_bin_view,
            texture_cache: &mut self.cache_manager.texture_cache,
            loading_set: &mut self.cache_manager.loading_set,
            loading_icons: &mut self.loading_icons,
            failed_icons: &self.failed_icons,
            scanned_folders: &mut self.scanned_folders,
            folder_icon_texture: folder_icon_texture.as_ref(),
            computer_icon: computer_icon.as_ref(),
            drive_icon_cache: &mut self.cache_manager.drive_icon_cache,
            item_icon_loader: &mut self.item_icon_loader,
            folder_preview_cache: &mut self.cache_manager.folder_preview_cache,
            folder_preview_loading: &mut self.cache_manager.folder_preview_loading,
            pending_ops: &mut self.pending_ops,
            failed_thumbnails: &self.cache_manager.failed_thumbnails,
            scroll_offset_y: self.scroll_offset_y,
            mut_scroll_offset_y: &mut self.scroll_offset_y,
            last_input: self.last_input,
            scroll_predictor: &mut self.scroll_predictor,
            last_scroll_time: &mut self.last_scroll_time,
            last_scroll_offset: &mut self.last_scroll_offset,
            pending_upload_set: &mut self.cache_manager.pending_upload_set,
            is_video_docked_visible,
            prefetch_rows,
            visible_index_range: &mut self.visible_index_range,
        };

        // Use a different approach: collect actions in vectors
        let mut actions = Vec::new();

        let mut ops = GridOps {
            actions: &mut actions,
        };

        let action = grid_view::render_grid_view(ui, &mut ctx, &mut ops);

        // Update state from context
        self.last_grid_cols = ctx.last_grid_cols;
        self.renaming_state = ctx.renaming_state;
        // Always consume focus_rename after one frame (cursor selection applied once)
        self.focus_rename = false;

        // Process actions (blocked during renaming, except click on item itself)
        let is_renaming = self.renaming_state.is_some();
        match action {
            Some(grid_view::GridViewAction::Click(idx)) if !is_renaming => {
                if let Some(item) = self.items.get(idx) {
                    let ctrl = ui.input(|i| i.modifiers.ctrl);
                    let shift = ui.input(|i| i.modifiers.shift);

                    if ctrl {
                        // Ctrl + Click: Toggle and set focus/anchor
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
                            for i in start..=end {
                                if let Some(it) = self.items.get(i) {
                                    self.multi_selection.insert(it.path.clone());
                                }
                            }
                            self.selected_item = Some(idx);
                            self.selected_file = Some(item.clone());
                        } else {
                            // Fallback
                            self.multi_selection.insert(item.path.clone());
                            self.selected_item = Some(idx);
                            self.selection_anchor = Some(idx);
                            self.selected_file = Some(item.clone());
                        }
                    } else {
                        // Simple Click: Reset selection and set focus/anchor
                        self.multi_selection.clear();
                        self.multi_selection.insert(item.path.clone());
                        self.selected_item = Some(idx);
                        self.selection_anchor = Some(idx);
                        self.selected_file = Some(item.clone());
                    }

                    self.update_selected_thumbnail();
                }
            }
            Some(grid_view::GridViewAction::DoubleClick(idx)) if !is_renaming => {
                let mut path_to_navigate = None;
                if let Some(item) = self.items.get(idx) {
                    if item.is_dir {
                        if !self.is_recycle_bin_view {
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
                            open_with_shell(&path);
                        }
                    }
                }

                if let Some(path) = path_to_navigate {
                    self.navigate_to(&path.to_string_lossy());
                }
            }
            Some(grid_view::GridViewAction::SecondaryClick(idx)) if !is_renaming => {
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
            Some(grid_view::GridViewAction::EmptyAreaSecondaryClick) if !is_renaming => {
                let path = PathBuf::from(&self.current_path);
                let pointer_pos = ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO);
                let right_bound = ui.available_rect_before_wrap().right();
                self.populate_context_menu(ui.ctx(), &[path.clone()], true, None);
                self.context_menu
                    .open(pointer_pos, right_bound, None, vec![path], true);
            }
            _ => {}
        }

        // Execute collected actions
        for action in actions {
            match action {
                GridAction::NavigateTo(path) => self.navigate_to(&path),
                GridAction::OpenWithShell(path) => open_with_shell(&path),
                GridAction::RequestThumbnailLoad(path, size, modified) => {
                    self.request_thumbnail_load_with_modified(path, size, modified)
                }
                GridAction::RequestThumbnailLoadWithIndex(path, size, index, modified) => {
                    self.request_thumbnail_load_with_index_and_modified(path, size, index, modified)
                }
                GridAction::RequestFolderScan(path) => self.request_folder_scan(path),
                GridAction::RequestFolderPreviewLoad(path) => {
                    self.request_folder_preview_load(path)
                }
                GridAction::RequestThumbnailPrefetch(path, size, modified) => {
                    self.request_thumbnail_prefetch_with_index_and_modified(path, size, 0, modified)
                }
                GridAction::RequestThumbnailPrefetchWithIndex(path, size, index, modified) => {
                    self.request_thumbnail_prefetch_with_index_and_modified(path, size, index, modified)
                }
                GridAction::RequestIconLoad(path) => self.request_icon_load(path),
                GridAction::RenameWithShell(idx) => self.rename_with_shell(idx),
                GridAction::NotifyIdleVisibleItems(items) => {
                    let _ = self
                        .idle_warmup_sender
                        .send(crate::workers::idle_warmup::IdleWarmupMessage::VisibleItems(items));
                }
            }
        }

        // Reset scroll trigger after view has consumed it
        self.scroll_to_selected = false;
    }
}
