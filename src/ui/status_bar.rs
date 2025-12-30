//! Status bar rendering for the file manager.
//!
//! This module contains the rendering logic for the application status bar.

use eframe::egui;

use crate::ui::app::ImageViewerApp;

impl ImageViewerApp {
    /// Renders the application status bar.
    pub fn render_status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Left side: item count and loading status
            if self.is_loading_folder {
                ui.spinner();
                ui.label("Carregando...");
            } else {
                let item_text = if self.total_items == 1 {
                    "1 item".to_string()
                } else {
                    format!("{} itens", self.total_items)
                };
                ui.label(item_text);
            }
            
            // Center: view mode and thumbnail size
            ui.with_layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
                ui.label("Modo:");
                if ui.selectable_label(self.view_mode == crate::domain::file_entry::ViewMode::Grid, "Grade").clicked() {
                    self.view_mode = crate::domain::file_entry::ViewMode::Grid;
                }
                if ui.selectable_label(self.view_mode == crate::domain::file_entry::ViewMode::List, "Lista").clicked() {
                    self.view_mode = crate::domain::file_entry::ViewMode::List;
                }
                
                ui.separator();
                
                ui.label("Tamanho:");
                ui.add(egui::Slider::new(&mut self.thumbnail_size, 64.0..=256.0).show_value(false));
            });
            
            // Right side: sort mode
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label("Ordenar por:");
                
                let sort_modes = [
                    (crate::domain::file_entry::SortMode::Name, "Nome"),
                    (crate::domain::file_entry::SortMode::Date, "Data"),
                    (crate::domain::file_entry::SortMode::Size, "Tamanho"),
                ];
                
                for (mode, label) in sort_modes {
                    if ui.selectable_label(self.sort_mode == mode, label).clicked() {
                        if self.sort_mode == mode {
                            self.sort_descending = !self.sort_descending;
                        } else {
                            self.sort_mode = mode;
                            self.sort_descending = false;
                        }
                        self.sort_items();
                    }
                }
                
                // Sort direction indicator
                let arrow = if self.sort_descending { "↓" } else { "↑" };
                ui.label(arrow);
            });
        });
    }
}
