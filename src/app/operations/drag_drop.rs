//! Internal drag-and-drop for file/folder items (Explorer-like behavior).
//!
//! This module supports dragging selected items and dropping onto a folder item:
//! - `Ctrl` forces copy
//! - `Shift` forces move
//! - Without modifiers: move on same volume, copy across volumes

use crate::app::state::ImageViewerApp;
use eframe::egui;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragDropOperation {
    Copy,
    Move,
}

impl ImageViewerApp {
    /// Starts an item drag operation from the given index.
    pub fn begin_item_drag(&mut self, item_idx: usize) {
        if self.renaming_state.is_some() || self.is_computer_view || self.is_recycle_bin_view {
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
        self.drag_target_folder = None;
        self.ui_ctx.request_repaint();
    }

    /// Updates the current drop target based on hovered item index.
    pub fn update_item_drag_target_from_hover(&mut self, hovered_idx: Option<usize>) {
        if !self.is_item_dragging {
            self.drag_target_folder = None;
            return;
        }

        let target = hovered_idx
            .and_then(|idx| self.items.get(idx))
            .filter(|item| item.is_dir)
            .map(|item| item.path.clone());

        self.drag_target_folder = target.filter(|target| self.is_valid_drop_target(target));
    }

    /// Applies cursor feedback while dragging.
    pub fn apply_item_drag_cursor_feedback(
        &self,
        ctx: &egui::Context,
        ctrl_pressed: bool,
        shift_pressed: bool,
    ) {
        if !self.is_item_dragging {
            return;
        }

        if let Some(dest_folder) = self.drag_target_folder.as_ref() {
            match self.resolve_drag_operation(dest_folder, ctrl_pressed, shift_pressed) {
                DragDropOperation::Copy => ctx.set_cursor_icon(egui::CursorIcon::Copy),
                DragDropOperation::Move => ctx.set_cursor_icon(egui::CursorIcon::Move),
            }
        } else {
            // Neutral feedback while searching for a drop target (avoid "blocked" feel).
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        }

        ctx.request_repaint();
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

        self.file_ops_in_progress += 1;
        if self.file_op_sender.send(request).is_err() {
            self.file_ops_in_progress = self.file_ops_in_progress.saturating_sub(1);
        }

        self.is_item_dragging = false;
        self.drag_target_folder = None;
        self.ui_ctx.request_repaint();
    }

    /// Cancels any active drag state.
    pub fn cancel_item_drag(&mut self) {
        self.is_item_dragging = false;
        self.drag_payload_paths.clear();
        self.drag_target_folder = None;
    }

    fn collect_drag_payload(&self, item_idx: usize) -> Vec<PathBuf> {
        let Some(item) = self.items.get(item_idx) else {
            return Vec::new();
        };

        if self.multi_selection.contains(&item.path) && !self.multi_selection.is_empty() {
            let mut paths: Vec<PathBuf> = self.multi_selection.iter().cloned().collect();
            paths.retain(|path| self.items.iter().any(|it| it.path == *path));
            if paths.is_empty() {
                paths.push(item.path.clone());
            }
            return paths;
        }

        vec![item.path.clone()]
    }

    fn is_valid_drop_target(&self, target: &Path) -> bool {
        if self.drag_payload_paths.is_empty() {
            return false;
        }

        let target_norm = normalize_path_for_compare(target);

        for source in &self.drag_payload_paths {
            let source_norm = normalize_path_for_compare(source);

            // Can't drop onto itself.
            if source_norm == target_norm {
                return false;
            }

            // Can't drop a folder into itself/descendant.
            let source_prefix = format!("{source_norm}\\");
            if target_norm.starts_with(&source_prefix) {
                return false;
            }

            // No-op: dropping onto the current parent folder.
            if let Some(parent) = source.parent() {
                let parent_norm = normalize_path_for_compare(parent);
                if parent_norm == target_norm {
                    return false;
                }
            }
        }

        true
    }

    fn resolve_drag_operation(
        &self,
        dest_folder: &Path,
        ctrl_pressed: bool,
        shift_pressed: bool,
    ) -> DragDropOperation {
        if ctrl_pressed {
            return DragDropOperation::Copy;
        }
        if shift_pressed {
            return DragDropOperation::Move;
        }

        let target_volume = volume_key(dest_folder);
        let same_volume_for_all = self.drag_payload_paths.iter().all(|source| {
            let base = source.parent().unwrap_or(source.as_path());
            volume_key(base) == target_volume
        });

        if same_volume_for_all {
            DragDropOperation::Move
        } else {
            DragDropOperation::Copy
        }
    }
}

fn normalize_path_for_compare(path: &Path) -> String {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    if let Some(stripped) = lower.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        lower
    }
}

fn volume_key(path: &Path) -> Option<String> {
    path.components().find_map(|comp| match comp {
        Component::Prefix(prefix) => {
            Some(prefix.as_os_str().to_string_lossy().to_ascii_uppercase())
        }
        _ => None,
    })
}
