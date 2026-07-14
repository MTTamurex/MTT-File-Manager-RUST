use eframe::egui;

use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn render_column_list_view(&mut self, ui: &mut egui::Ui) {
        self.render_column_list_bridge(ui);
    }
}
