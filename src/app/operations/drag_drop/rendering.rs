use crate::app::state::ImageViewerApp;
use eframe::egui;

use super::validation::DragDropOperation;

impl ImageViewerApp {
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

            // Use pre-cached icon (loaded once in begin_item_drag) — no Shell calls per frame.
            let icon_texture = self.drag_icon_cache.clone();

            (display_name, icon_texture)
        } else {
            // Item not in current tab's list (cross-tab drag) — still use cached icon.
            (
                primary_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| primary_path.to_string_lossy().to_string()),
                self.drag_icon_cache.clone(),
            )
        };

        let total = self.drag_payload_paths.len();
        let op_label = self.drag_target_folder.as_ref().map(|dest| {
            match self.resolve_drag_operation(dest, ctrl_pressed, shift_pressed) {
                DragDropOperation::Copy => rust_i18n::t!("drag_drop.copy"),
                DragDropOperation::Move => rust_i18n::t!("drag_drop.move"),
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
}
