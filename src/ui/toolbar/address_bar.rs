use crate::ui::theme;
use eframe::egui;

pub(super) fn edit_rects(
    bar: egui::Rect,
    show_close: bool,
    spacing: f32,
) -> (egui::Rect, Option<egui::Rect>) {
    let mut text = bar.shrink2(egui::vec2(4.0, 1.0));
    let close = show_close.then(|| {
        let rect = egui::Rect::from_min_max(
            egui::pos2((text.right() - 22.0).max(text.left()), text.top()),
            text.max,
        );
        text.max.x = (rect.left() - spacing).max(text.left());
        rect
    });
    (text, close)
}

pub(super) fn render_centered_clear_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    tooltip: &str,
) -> egui::Response {
    let response = ui.interact(
        rect,
        ui.id().with("close_address_history"),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tooltip)
    });
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            3.0,
            if ui.visuals().dark_mode {
                theme::color_dark_hover()
            } else {
                theme::color_hover()
            },
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            3.0,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }

    let center = rect.center();
    let half = 4.0;
    let stroke = egui::Stroke::new(1.5, theme::secondary_text_color(ui.ctx()));
    ui.painter().line_segment(
        [
            center + egui::vec2(-half, -half),
            center + egui::vec2(half, half),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            center + egui::vec2(half, -half),
            center + egui::vec2(-half, half),
        ],
        stroke,
    );
    response.on_hover_text(tooltip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_and_close_button_fit_inside_bar_and_share_its_center() {
        for width in [100.0, 240.0, 800.0] {
            let bar = egui::Rect::from_min_size(egui::pos2(15.0, 30.0), egui::vec2(width, 26.0));
            let (text, close) = edit_rects(bar, true, 8.0);
            let close = close.unwrap();
            assert!(bar.contains_rect(text));
            assert!(bar.contains_rect(close));
            assert_eq!(text.center().y, bar.center().y);
            assert_eq!(close.center().y, bar.center().y);
            assert_eq!(close.left() - text.right(), 8.0);
            assert_eq!(close.width(), 22.0);
            let (full_text, close) = edit_rects(bar, false, 8.0);
            assert!(close.is_none());
            assert!(full_text.width() > text.width());
        }
    }
}
