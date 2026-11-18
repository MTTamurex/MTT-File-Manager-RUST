use eframe::egui::{self, Color32, Pos2, Rect};

pub(super) const CHIP_WIDTH: f32 = 124.0;

pub(super) fn chip_label(ui: &egui::Ui, label: &str) -> String {
    crate::ui::preview_panel::utils::truncate_text_to_fit(
        label,
        CHIP_WIDTH - 56.0,
        &egui::FontId::proportional(12.0),
        ui,
    )
}

pub(super) fn paint_chip(
    ui: &egui::Ui,
    rect: Rect,
    label: &str,
    icon: Option<&egui::TextureHandle>,
    selected: bool,
    hovered: bool,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }

    let fill = if selected {
        crate::ui::theme::selection_color(ui.ctx())
    } else if hovered {
        crate::ui::theme::selection_hover_color(ui.ctx())
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 5.0, fill);

    let mut text_x = rect.left() + 10.0;
    if let Some(icon) = icon {
        let size = icon.size_vec2();
        let icon_rect = Rect::from_center_size(
            Pos2::new(text_x + 8.0, rect.center().y),
            size * (16.0 / size.x.max(size.y).max(1.0)),
        );
        ui.painter().image(
            icon.id(),
            icon_rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        text_x += 22.0;
    } else {
        text_x += 22.0;
    }
    ui.painter().text(
        Pos2::new(text_x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.0),
        if selected {
            crate::ui::theme::selection_text_color(ui.ctx())
        } else {
            ui.visuals().text_color()
        },
    );
}

pub(super) fn paint_group_chip(ui: &egui::Ui, rect: Rect, label: &str, hovered: bool) {
    paint_chip(ui, rect, label, None, false, hovered);
    if ui.is_rect_visible(rect) {
        ui.painter().text(
            Pos2::new(rect.left() + 18.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            "▤",
            egui::FontId::proportional(14.0),
            Color32::from_gray(130),
        );
    }
}
