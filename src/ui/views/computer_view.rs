//! Computer view rendering (This PC)
//! Follows .cursorrules: single responsibility, < 300 lines

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Ui};

use crate::domain::file_entry::IconSize;

/// Context for computer view rendering
pub struct ComputerViewContext<'a> {
    pub disks: &'a [(String, String)], // (path, label)
    pub selected_disk: Option<&'a str>,
    pub computer_icon: Option<&'a egui::TextureHandle>,
    pub drive_icon_cache: &'a mut lru::LruCache<String, egui::TextureHandle>,
}

/// Operations that can be performed from computer view
pub trait ComputerViewOperations {
    fn navigate_to(&mut self, path: &str);
    fn extract_drive_icon(
        &mut self,
        drive_path: &str,
        size: IconSize,
    ) -> Option<egui::TextureHandle>;
}

/// Renders the computer view (This PC)
pub fn render_computer_view(
    ui: &mut Ui,
    ctx: &mut ComputerViewContext,
    _ops: &mut dyn ComputerViewOperations,
) -> Option<String> {
    let mut clicked_disk = None;

    for (disk_path, disk_label) in ctx.disks {
        // Preload drive icon if not in cache — delegate to ops (non-blocking)
        let drive_icon = if let Some(icon) = ctx.drive_icon_cache.get(disk_path) {
            Some(icon.clone())
        } else {
            // Non-blocking: ops implementation must use async extraction (e.g. IconLoader)
            _ops.extract_drive_icon(disk_path, IconSize::Small)
        };

        // Render drive with icon + label using interact() for full cursor control
        let is_selected = ctx.selected_disk == Some(disk_path.as_str());

        // Draw content in horizontal layout
        let (mut rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 24.0),
            Sense::click(), // Capture clicks, no selectable text
        );

        // Expand rect to fill entire sidebar width (remove gaps)
        rect.min.x = ui.clip_rect().min.x;
        rect.max.x = ui.clip_rect().max.x;

        // Only draw if visible
        if ui.is_rect_visible(rect) {
            // Selection background
            if is_selected {
                ui.painter().rect_filled(
                    rect,
                    0.0, // No rounded corners to stay flush with edges
                    Color32::from_rgb(200, 220, 240),
                );
            }

            // Hover effect
            if response.hovered() && !is_selected {
                ui.painter().rect_filled(
                    rect,
                    2.0,
                    Color32::from_rgba_unmultiplied(200, 220, 240, 50),
                );
            }

            // Draw icon and text manually
            let mut cursor_x = rect.min.x + 5.0;

            // Icon
            if let Some(icon) = drive_icon {
                let icon_rect = Rect::from_min_size(
                    Pos2::new(cursor_x, rect.center().y - 8.0),
                    egui::vec2(16.0, 16.0),
                );
                ui.painter().image(
                    icon.id(),
                    icon_rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
                cursor_x += 20.0;
            } else {
                ui.painter().text(
                    Pos2::new(cursor_x, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    "💽",
                    egui::FontId::proportional(14.0),
                    ui.visuals().text_color(),
                );
                cursor_x += 20.0;
            }

            // Texto
            ui.painter().text(
                Pos2::new(cursor_x, rect.center().y),
                egui::Align2::LEFT_CENTER,
                disk_label,
                egui::FontId::proportional(14.0),
                if is_selected {
                    Color32::from_rgb(0, 50, 100)
                } else {
                    ui.visuals().text_color()
                },
            );
        }

        if response.clicked() {
            clicked_disk = Some(disk_path.clone());
        }

        ui.add_space(3.0);
    }

    clicked_disk
}
