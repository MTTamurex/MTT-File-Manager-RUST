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
        // Already dragging – don't restart (avoids resetting target/hover every frame)
        if self.is_item_dragging {
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
        self.drag_source_folder = Some(PathBuf::from(&self.navigation_state.current_path));
        self.drag_target_folder = None;
        self.drag_hovered_folder = None;
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

    /// Applies cursor feedback while dragging.
    pub fn apply_item_drag_cursor_feedback(&self, ctx: &egui::Context) {
        if !self.is_item_dragging {
            return;
        }

        if self.drag_target_folder.is_some() {
            // Over a valid drop target → show Grab cursor
            ctx.set_cursor_icon(egui::CursorIcon::Grab);
        } else if self.drag_hovered_folder.is_some() {
            // Hovering over a specific folder that was rejected → NotAllowed
            ctx.set_cursor_icon(egui::CursorIcon::NotAllowed);
        } else {
            // Not over any folder (empty space, tab bar, files, etc.) → default cursor
            ctx.set_cursor_icon(egui::CursorIcon::Default);
        }

        ctx.request_repaint();
    }

    /// Renders the drag ghost near the pointer (icon + item name/count).
    pub fn render_item_drag_preview(
        &mut self,
        ctx: &egui::Context,
        ctrl_pressed: bool,
        shift_pressed: bool,
    ) {
        if !self.is_item_dragging {
            return;
        }

        // Use latest_pos (tracks current mouse position) instead of interact_pos
        // (which may return the initial press position during a drag).
        let pointer_pos = ctx
            .pointer_latest_pos()
            .or_else(|| ctx.input(|i| i.pointer.interact_pos()));
        let Some(pointer_pos) = pointer_pos else {
            return;
        };

        let Some(primary_path) = self.drag_payload_paths.first().cloned() else {
            return;
        };

        let primary_item = self
            .items
            .iter()
            .find(|it| it.path == primary_path)
            .cloned();
        let (display_name, icon_texture) = if let Some(item) = primary_item {
            let display_name = if item.name.is_empty() {
                item.path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| item.path.to_string_lossy().to_string())
            } else {
                item.name.clone()
            };

            // allow_blocking = true: drag preview is a single icon (not in scroll loop),
            // so blocking Shell API calls are acceptable for reliability.
            let icon_texture = if item.drive_info.is_some() {
                self.item_icon_loader
                    .get_or_load_drive_icon(ctx, &item.path.to_string_lossy())
            } else if item.is_dir && !item.is_archive() {
                self.item_icon_loader
                    .get_or_load_icon(ctx, &item.path, true, true)
                    .or_else(|| self.cache_manager.folder_icon_texture.clone())
            } else if item.is_media() {
                self.cache_manager
                    .texture_cache
                    .get(&item.path)
                    .cloned()
                    .or_else(|| {
                        self.item_icon_loader
                            .get_or_load_icon(ctx, &item.path, false, true)
                    })
            } else {
                self.item_icon_loader
                    .get_or_load_icon(ctx, &item.path, false, true)
            };

            (display_name, icon_texture)
        } else {
            (
                primary_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| primary_path.to_string_lossy().to_string()),
                None,
            )
        };

        let total = self.drag_payload_paths.len();
        let op_label = self.drag_target_folder.as_ref().map(|dest| {
            match self.resolve_drag_operation(dest, ctrl_pressed, shift_pressed) {
                DragDropOperation::Copy => "Copiar",
                DragDropOperation::Move => "Mover",
            }
        });

        // Build label
        let mut label = display_name;
        if total > 1 {
            label = format!("{label} (+{})", total - 1);
        }
        if label.chars().count() > 36 {
            label = format!("{}...", label.chars().take(36).collect::<String>());
        }

        // --- Paint drag ghost directly via top-level painter (most reliable) ---
        let layer_id = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("drag_ghost_layer"));
        let painter = ctx.layer_painter(layer_id);

        let icon_size = 20.0;
        let padding = 8.0;
        let spacing = 6.0;
        let font_id = egui::FontId::proportional(12.5);
        let op_font_id = egui::FontId::proportional(11.0);

        // Measure text
        let galley = painter.layout_no_wrap(label.clone(), font_id.clone(), egui::Color32::BLACK);
        let text_width = galley.size().x;
        let text_height = galley.size().y;

        let mut total_width = padding + icon_size + spacing + text_width + padding;
        let op_galley = op_label.as_ref().map(|op| {
            let g = painter.layout_no_wrap(
                op.to_string(),
                op_font_id.clone(),
                egui::Color32::from_rgb(24, 122, 255),
            );
            total_width += spacing + g.size().x;
            g
        });

        let box_height = padding + icon_size.max(text_height) + padding;
        let origin = pointer_pos + egui::vec2(16.0, 18.0);
        let box_rect = egui::Rect::from_min_size(origin, egui::vec2(total_width, box_height));

        // Background with shadow
        let shadow_offset = egui::vec2(1.0, 2.0);
        let shadow_rect = box_rect.translate(shadow_offset);
        painter.rect_filled(shadow_rect, 6.0, egui::Color32::from_black_alpha(30));
        painter.rect_filled(
            box_rect,
            6.0,
            egui::Color32::from_rgba_unmultiplied(250, 250, 250, 240),
        );
        painter.rect_stroke(
            box_rect,
            6.0,
            egui::Stroke::new(1.0, egui::Color32::from_gray(200)),
            egui::StrokeKind::Outside,
        );

        // Icon
        let icon_rect = egui::Rect::from_min_size(
            origin + egui::vec2(padding, (box_height - icon_size) / 2.0),
            egui::vec2(icon_size, icon_size),
        );
        if let Some(icon) = &icon_texture {
            painter.image(
                icon.id(),
                icon_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            painter.text(
                icon_rect.center(),
                egui::Align2::CENTER_CENTER,
                "📄",
                egui::FontId::proportional(14.0),
                egui::Color32::GRAY,
            );
        }

        // Text label
        let text_pos = egui::pos2(
            icon_rect.right() + spacing,
            origin.y + (box_height - text_height) / 2.0,
        );
        painter.galley(text_pos, galley, egui::Color32::BLACK);

        // Operation label (Copiar/Mover)
        if let Some(op_g) = op_galley {
            let op_pos = egui::pos2(
                text_pos.x + text_width + spacing,
                origin.y + (box_height - op_g.size().y) / 2.0,
            );
            painter.galley(op_pos, op_g, egui::Color32::from_rgb(24, 122, 255));
        }
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
        self.drag_target_folder = None;
        self.drag_hovered_folder = None;

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
        self.drag_source_folder = None;
        self.drag_target_folder = None;
        self.drag_hovered_folder = None;
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
        }

        // No-op: reject if ALL sources are already direct children of the target folder.
        let all_already_in_target = self.drag_payload_paths.iter().all(|source| {
            source
                .parent()
                .is_some_and(|p| normalize_path_for_compare(p) == target_norm)
        });
        if all_already_in_target {
            return false;
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
