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
                crate::ui::theme::palette::hover(ui.ctx(), Color32::from_gray(48))
            } else {
                Color32::from_gray(242)
            },
        );
    }

    let text_color = if ui.visuals().dark_mode {
        crate::ui::theme::current_palette(ui.ctx()).map_or(Color32::from_gray(195), |p| p.text)
    } else {
        Color32::from_gray(40)
    };
    let secondary = crate::ui::theme::current_palette(ui.ctx())
        .map_or(Color32::from_gray(125), |p| p.secondary);
    let left = rect.left() + 8.0;
    crate::ui::disclosure::paint(
        ui.painter(),
        egui::pos2(left + 6.0, rect.center().y),
        !collapsed,
        crate::ui::disclosure::GROUP_SIZE,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_headers_use_the_larger_shared_triangle_without_changing_the_hit_box() {
        for collapsed in [false, true] {
            let ctx = egui::Context::default();
            let mut header = Rect::NOTHING;
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                header = ui
                    .allocate_exact_size(egui::vec2(420.0, GROUP_HEADER_HEIGHT), Sense::hover())
                    .0;
                let response =
                    render_group_header(ui, header, &GroupKey::LocalDrives, 2, collapsed);
                assert_eq!(response.rect, header);
            });
            let triangle = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Path(path) if path.closed && path.points.len() == 3 => Some(path),
                    _ => None,
                })
                .expect("group header has a triangle");
            let expected = crate::ui::disclosure::points(
                egui::pos2(header.left() + 14.0, header.center().y),
                !collapsed,
                crate::ui::disclosure::GROUP_SIZE,
            );
            assert_eq!(triangle.points, expected);
            assert!(
                Rect::from_points(&triangle.points).width() > crate::ui::disclosure::SIDEBAR_SIZE
            );
        }
    }
}
