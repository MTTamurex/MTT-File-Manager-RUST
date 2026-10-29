//! Alpha-capable rendering is shared by all eframe viewports, including the root.
use eframe::egui;

pub(crate) fn paint_root_background(ui: &egui::Ui) {
    // eframe's previous clear color came from unmultiplied RGBA (12, 12, 12, 180).
    // Preserve those RGB values while keeping the main window fully opaque;
    // only the auxiliary tab viewport should have a transparent background.
    let previous = egui::Color32::from_rgba_unmultiplied(12, 12, 12, 180);
    let opaque = egui::Color32::from_rgb(previous.r(), previous.g(), previous.b());
    ui.painter().rect_filled(ui.max_rect(), 0.0, opaque);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_window_preserves_its_previous_rgb_without_becoming_transparent() {
        let ctx = egui::Context::default();
        let mut bounds = egui::Rect::NOTHING;
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            bounds = ui.max_rect();
            paint_root_background(ui);
        });
        let egui::Shape::Rect(background) = &output.shapes[0].shape else {
            panic!("expected root background");
        };
        let previous = egui::Color32::from_rgba_unmultiplied(12, 12, 12, 180);
        assert_eq!(background.rect, bounds);
        assert_eq!(background.fill.r(), previous.r());
        assert_eq!(background.fill.g(), previous.g());
        assert_eq!(background.fill.b(), previous.b());
        assert_eq!(background.fill.a(), u8::MAX);
    }
}
