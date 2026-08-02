use crate::application::grouping::{group_label, GroupKey};
use eframe::egui::{self, Color32, Rect, Sense, Stroke, Ui};

pub const GROUP_HEADER_HEIGHT: f32 = 30.0;
pub const GROUP_GAP: f32 = 6.0;

pub fn render_group_header(
    ui: &mut Ui,
    rect: Rect,
    key: &GroupKey,
    count: usize,
    collapsed: bool,
) -> egui::Response {
    let response = ui.interact(rect, ui.id().with(("group_header", key)), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            3.0,
            if ui.visuals().dark_mode {
                Color32::from_gray(48)
            } else {
                Color32::from_gray(242)
            },
        );
    }

    let text_color = if ui.visuals().dark_mode {
        Color32::from_gray(195)
    } else {
        Color32::from_gray(40)
    };
    let secondary = Color32::from_gray(125);
    let chevron = if collapsed { ">" } else { "v" };
    let left = rect.left() + 8.0;
    ui.painter().text(
        egui::pos2(left, rect.center().y),
        egui::Align2::LEFT_CENTER,
        chevron,
        egui::FontId::monospace(13.0),
        secondary,
    );
    let title = group_label(key);
    let title_pos = egui::pos2(left + 20.0, rect.center().y);
    ui.painter().text(
        title_pos,
        egui::Align2::LEFT_CENTER,
        &title,
        egui::FontId::proportional(13.0),
        text_color,
    );
    let title_width = ui
        .painter()
        .layout_no_wrap(title, egui::FontId::proportional(13.0), text_color)
        .size()
        .x;
    let count_text = format!("({count})");
    let count_pos = egui::pos2(title_pos.x + title_width + 7.0, rect.center().y);
    ui.painter().text(
        count_pos,
        egui::Align2::LEFT_CENTER,
        count_text,
        egui::FontId::proportional(12.0),
        secondary,
    );
    let line_start = (count_pos.x + 70.0).min(rect.right() - 8.0);
    if line_start < rect.right() - 8.0 {
        ui.painter().line_segment(
            [
                egui::pos2(line_start, rect.center().y),
                egui::pos2(rect.right() - 8.0, rect.center().y),
            ],
            Stroke::new(1.0, secondary.gamma_multiply(0.35)),
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
