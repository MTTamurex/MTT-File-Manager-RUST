use crate::app::copy_panel_state::TransferMetrics;
use crate::ui::theme;
use eframe::egui::{self, Stroke};

pub(super) fn render(ui: &mut egui::Ui, metrics: &TransferMetrics) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 56.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
    let accent = theme::accent_color(ui.ctx());
    for line in 1..4 {
        let y = egui::lerp(rect.y_range(), line as f32 / 4.0);
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            Stroke::new(0.5, ui.visuals().widgets.noninteractive.bg_stroke.color),
        );
    }
    let maximum = metrics.history.iter().copied().fold(1.0f32, f32::max);
    let points: Vec<_> = metrics
        .history
        .iter()
        .enumerate()
        .map(|(index, value)| {
            egui::pos2(
                rect.left() + rect.width() * index as f32 / 119.0,
                rect.bottom() - 3.0 - (rect.height() - 6.0) * value / maximum,
            )
        })
        .collect();
    if points.len() >= 2 {
        painter.add(egui::Shape::line(points, Stroke::new(1.8, accent)));
    }
}
