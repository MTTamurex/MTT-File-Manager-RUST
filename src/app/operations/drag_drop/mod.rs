//! Internal drag-and-drop for file/folder items (Explorer-like behavior).
//!
//! This module supports dragging selected items and dropping onto a folder item:
//! - `Ctrl` forces copy
//! - `Shift` forces move
//! - Without modifiers: move on same volume, copy across volumes

mod rendering;
mod validation;

use crate::app::state::ImageViewerApp;
use std::path::PathBuf;

use validation::{normalize_path_for_compare, DragDropOperation};

impl ImageViewerApp {
    /// Starts an item drag operation from the given index.
    pub fn begin_item_drag(&mut self, item_idx: usize) {
        // Already dragging – don't restart (avoids resetting target/hover every frame)
        if self.is_item_dragging {
            return;
        }

        if !self
            .ui_ctx
            .input(|i| i.pointer.button_down(eframe::egui::PointerButton::Primary))
        {
            return;
        }

        if self.renaming_state.is_some()
            || self.navigation_state.is_computer_view
            || self.navigation_state.is_recycle_bin_view
        {
            return;
        }

        let Some(item) = self.items.get(item_idx).cloned() else {
            return;
        };

        let mut payload = self.collect_drag_payload(item_idx);
        if payload.is_empty() {
            return;
        }

        if !payload.iter().any(|p| p == &item.path) {
            payload.push(item.path.clone());
        }

        let payload_is_single_directory = payload.len() == 1
            && self
                .items
                .iter()
                .find(|candidate| candidate.path == payload[0])
                .is_some_and(|candidate| candidate.is_dir);

        // Explorer-like behavior: dragging a non-selected item turns it into single selection.
        if !self.multi_selection.contains(&item.path) {
            self.multi_selection.clear();
            self.multi_selection.insert(item.path.clone());
            self.selected_item = Some(item_idx);
            self.selection_anchor = Some(item_idx);
            self.selected_file = Some(item);
            self.update_selected_thumbnail();
        }

        self.is_item_dragging = true;
        self.drag_payload_paths = payload;
        self.drag_payload_is_single_directory = payload_is_single_directory;
        self.drag_source_folder = Some(PathBuf::from(&self.navigation_state.current_path));
        self.drag_target_folder = None;
        self.drag_hovered_folder = None;

        // Pre-cache the drag icon once — avoids blocking Shell calls every frame in render.
        let ui_ctx = self.ui_ctx.clone();
        let primary_item = self
            .drag_payload_paths
            .first()
            .and_then(|p| self.items.iter().find(|it| &it.path == p))
            .cloned();
        self.drag_icon_cache = if let Some(ref primary) = primary_item {
            if primary.drive_info.is_some() {
                self.item_icon_loader
                    .get_or_load_drive_icon(&ui_ctx, &primary.path.to_string_lossy())
            } else if primary.is_dir && !primary.is_archive() {
                let from_loader =
                    self.item_icon_loader
                        .get_or_load_icon(&ui_ctx, &primary.path, true, true);
                from_loader.or_else(|| self.cache_manager.folder_icon_texture.clone())
            } else if primary.is_media() {
                let from_cache = self.cache_manager.texture_cache.get(&primary.path).cloned();
                from_cache.or_else(|| {
                    self.item_icon_loader
                        .get_or_load_icon(&ui_ctx, &primary.path, false, true)
                })
            } else {
                self.item_icon_loader
                    .get_or_load_icon(&ui_ctx, &primary.path, false, true)
            }
        } else {
            None
        };

        self.ui_ctx.request_repaint();
    }

    /// Updates the current drop target based on hovered item index.
    pub fn update_item_drag_target_from_hover(&mut self, hovered_idx: Option<usize>) {
        if !self.is_item_dragging {
            self.drag_target_folder = None;
            self.drag_hovered_folder = None;
            return;
        }

        let hovered_folder = hovered_idx
            .and_then(|idx| self.items.get(idx))
            .filter(|item| item.is_dir)
            .map(|item| item.path.clone());

        self.drag_hovered_folder = hovered_folder.clone();

        // If hovering over a specific folder, try that as the target.
        // Otherwise, fall back to the current directory as the drop target,
        // but only when we're NOT in the drag's source folder (to allow
        // dropping onto the open folder of a different tab).
        let candidate = hovered_folder.or_else(|| {
            if self.navigation_state.current_path.is_empty() {
                return None;
            }
            let cur = PathBuf::from(&self.navigation_state.current_path);
            // Don't fall back to the source folder (items are already there)
            if let Some(ref src) = self.drag_source_folder {
                if normalize_path_for_compare(src) == normalize_path_for_compare(&cur) {
                    return None;
                }
            }
            Some(cur)
        });

        self.drag_target_folder = candidate.filter(|target| self.is_valid_drop_target(target));
    }

    /// Completes an in-progress drag operation (drop).
    pub fn complete_item_drag(&mut self, ctrl_pressed: bool, shift_pressed: bool) {
        if !self.is_item_dragging {
            return;
        }

        let Some(dest_folder) = self.drag_target_folder.clone() else {
            self.cancel_item_drag();
            return;
        };

        if !self.is_valid_drop_target(&dest_folder) {
            self.cancel_item_drag();
            return;
        }

        let paths = std::mem::take(&mut self.drag_payload_paths);
        if paths.is_empty() {
            self.cancel_item_drag();
            return;
        }

        let hwnd = self.native_hwnd.unwrap_or_default();
        let request = match self.resolve_drag_operation(&dest_folder, ctrl_pressed, shift_pressed) {
            DragDropOperation::Copy => {
                crate::workers::file_operation_worker::FileOperationRequest::copy_batch(
                    paths,
                    dest_folder,
                    hwnd,
                )
            }
            DragDropOperation::Move => {
                crate::workers::file_operation_worker::FileOperationRequest::move_batch(
                    paths,
                    dest_folder,
                    hwnd,
                )
            }
        };

        self.file_operation_state.file_ops_in_progress += 1;
        if self
            .file_operation_state
            .file_op_sender
            .send(request)
            .is_err()
        {
            self.file_operation_state.file_ops_in_progress = self
                .file_operation_state
                .file_ops_in_progress
                .saturating_sub(1);
        }

        // Clear drag state
        self.is_item_dragging = false;
        self.drag_payload_is_single_directory = false;
        self.drag_target_folder = None;
        self.drag_hovered_folder = None;
        self.drag_icon_cache = None;

        // Clear selection so the detail panel updates to show folder info
        // instead of stale references to the moved/copied items.
        self.multi_selection.clear();
        self.selected_item = None;
        self.selected_file = None;
        self.selected_thumbnail = None;
        self.selected_metadata = None;

        // Also clear selection in the source tab's saved state.
        // After a tab switch the source tab's selection was persisted via sync_to_tab,
        // so clearing only the current (destination) app state isn't enough.
        if let Some(ref src) = self.drag_source_folder {
            let src_norm = normalize_path_for_compare(src);
            let active_idx = self.tab_manager.active_tab;
            for (i, tab) in self.tab_manager.tabs.iter_mut().enumerate() {
                if i != active_idx
                    && normalize_path_for_compare(&std::path::PathBuf::from(&tab.path)) == src_norm
                {
                    tab.multi_selection.clear();
                    tab.selected_item = None;
                    tab.selected_file = None;
                    tab.selected_thumbnail = None;
                    tab.selected_metadata = None;
                }
            }
        }
        self.drag_source_folder = None;

        self.ui_ctx.request_repaint();
    }

    /// Cancels any active drag state.
    pub fn cancel_item_drag(&mut self) {
        self.is_item_dragging = false;
        self.drag_payload_paths.clear();
        self.drag_payload_is_single_directory = false;
        self.drag_source_folder = None;
        self.drag_target_folder = None;
        self.drag_hovered_folder = None;
        self.drag_icon_cache = None;
    }
}
