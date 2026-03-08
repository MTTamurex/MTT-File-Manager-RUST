use super::*;
use rust_i18n::t;

/// Renders a drive slot (This PC)
pub(super) fn render_drive_slot(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    ctx: &mut ItemSlotContext,
    drive_info: &crate::domain::file_entry::DriveInfo,
) {
    let item = ctx.item;

    // Load real drive icon
    let drive_icon = ctx
        .icon_loader
        .get_or_load_drive_icon(ui.ctx(), &item.path.to_string_lossy());

    // GEOMETRY
    let available_h = rect.height();
    let available_w = rect.width();
    let icon_size = (ctx.thumbnail_size * 0.4).min(available_w * 0.5);
    let progress_w = (available_w * 0.8).min(150.0);
    let text_height = 36.0; // Name + Free Space
    let content_h = icon_size + 12.0 + 8.0 + text_height; // Icon + Bar + Padding + Text

    let vertical_margin = ((available_h - content_h) / 2.0).max(2.0);

    // Use `rect` as base for calculation instead of allocating space
    let start_y = rect.top() + vertical_margin;
    let mut current_y = start_y;

    // 1. ICON
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, current_y + icon_size / 2.0),
        egui::vec2(icon_size, icon_size),
    );

    if let Some(tex) = drive_icon {
        ui.put(
            icon_rect,
            egui::Image::new(&tex)
                .max_size(egui::vec2(icon_size, icon_size))
                .maintain_aspect_ratio(true),
        );
    } else {
        ui.painter().text(
            icon_rect.center(),
            egui::Align2::CENTER_CENTER,
            "💽",
            egui::FontId::proportional(icon_size * 0.8),
            egui::Color32::GRAY,
        );
    }

    current_y += icon_size + 8.0;

    // 2. PROGRESS BAR (Used Space)
    if drive_info.total_space > 0 {
        let bar_rect = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, current_y + 6.0),
            egui::vec2(progress_w, 12.0),
        );

        let used_space = drive_info.total_space - drive_info.free_space;
        let usage_ratio = used_space as f32 / drive_info.total_space as f32;

        // Bar color: blue or red if nearly full (> 90%)
        let bar_color = if usage_ratio > 0.9 {
            egui::Color32::from_rgb(230, 50, 50) // Red
        } else {
            egui::Color32::from_rgb(30, 130, 230) // Windows Blue
        };

        let bg_color = egui::Color32::from_gray(230);

        ui.painter().rect_filled(bar_rect, 2.0, bg_color);

        let filled_w = progress_w * usage_ratio;
        let filled_rect = egui::Rect::from_min_size(bar_rect.min, egui::vec2(filled_w, 12.0));
        ui.painter().rect_filled(filled_rect, 2.0, bar_color);

        // Add hover interaction for the bar
        ui.interact(bar_rect, ui.id().with("drive_bar"), egui::Sense::hover());
    }

    current_y += 12.0 + 6.0;

    // 3. TEXT (Name and Free Space)
    // Label for Name
    let name_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, current_y + 9.0), // ~half text height
        egui::vec2(progress_w, 18.0),
    );

    ui.put(
        name_rect,
        egui::Label::new(egui::RichText::new(super::display_name_for_item(item).as_ref()).size(11.0).strong()).truncate(),
    );

    current_y += 18.0;

    if drive_info.total_space > 0 {
        let free_gb = drive_info.free_space as f64 / (1024.0 * 1024.0 * 1024.0);
        let total_gb = drive_info.total_space as f64 / (1024.0 * 1024.0 * 1024.0);

        let (free_val, unit) = if total_gb >= 1000.0 {
            (free_gb / 1024.0, "TB")
        } else {
            (free_gb, "GB")
        };

        let (total_val, total_unit) = if total_gb >= 1000.0 {
            (total_gb / 1024.0, "TB")
        } else {
            (total_gb, "GB")
        };

        let details_rect = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, current_y + 9.0),
            egui::vec2(progress_w, 18.0),
        );

        ui.put(
            details_rect,
            egui::Label::new(
                egui::RichText::new(t!("drive_slot.free_of",
                    free_val = format!("{:.1}", free_val),
                    free_unit = unit,
                    total_val = format!("{:.1}", total_val),
                    total_unit = total_unit
                ).to_string())
                .size(9.0)
                .color(egui::Color32::from_gray(100)),
            )
            .truncate(),
        );
    }
}
