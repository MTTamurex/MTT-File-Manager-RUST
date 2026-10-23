//! Minimum allocations for manually painted UI text. Measurement uses the
//! active font, so rendering, hit-testing and virtualization share its metrics.

use eframe::egui;

pub(crate) fn row_height(ui: &egui::Ui, minimum: f32, size: f32, padding: f32) -> f32 {
    let height = ui.fonts_mut(|fonts| fonts.row_height(&egui::FontId::proportional(size)));
    minimum.max(height.ceil() + padding)
}
