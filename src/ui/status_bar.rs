//! Status bar rendering for the file manager.
//!
//! This module contains the rendering logic for the application status bar.

use eframe::egui;
use crate::domain::file_entry::{ViewMode, SortMode};

/// Status bar action that needs to be handled by the caller
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StatusBarAction {
    /// Sort mode or direction changed
    SortChanged,
    /// View mode changed
    ViewModeChanged,
    /// No action
    None,
}

/// Renders the application status bar.
/// Returns an action that needs to be handled by the caller.
pub fn render_status_bar(
    ui: &mut egui::Ui,
    is_loading_folder: &mut bool,
    total_items: usize,
    view_mode: &mut ViewMode,
    thumbnail_size: &mut f32,
    sort_mode: &mut SortMode,
    sort_descending: &mut bool,
) -> StatusBarAction {
    let mut action = StatusBarAction::None;
    
    ui.horizontal(|ui| {
        // Left side: item count and loading status
        if *is_loading_folder {
            ui.spinner();
            ui.label("Carregando...");
        } else {
            let item_text = if total_items == 1 {
                "1 item".to_string()
            } else {
                format!("{} itens", total_items)
            };
            ui.label(item_text);
        }
        
        // Center: view mode and thumbnail size
        ui.with_layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
            ui.label("Modo:");
            if ui.selectable_label(*view_mode == ViewMode::Grid, "Grade").clicked() {
                *view_mode = ViewMode::Grid;
                action = StatusBarAction::ViewModeChanged;
            }
            if ui.selectable_label(*view_mode == ViewMode::List, "Lista").clicked() {
                *view_mode = ViewMode::List;
                action = StatusBarAction::ViewModeChanged;
            }
            
            ui.separator();
            
            ui.label("Tamanho:");
            ui.add(egui::Slider::new(thumbnail_size, 64.0..=256.0).show_value(false));
        });
        
        // Right side: sort mode
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label("Ordenar por:");
            
            let sort_modes = [
                (SortMode::Name, "Nome"),
                (SortMode::Date, "Data"),
                (SortMode::Size, "Tamanho"),
            ];
            
            for (mode, label) in sort_modes {
                if ui.selectable_label(*sort_mode == mode, label).clicked() {
                    if *sort_mode == mode {
                        *sort_descending = !*sort_descending;
                    } else {
                        *sort_mode = mode;
                        *sort_descending = false;
                    }
                    action = StatusBarAction::SortChanged;
                }
            }
            
            // Sort direction indicator
            let arrow = if *sort_descending { "↓" } else { "↑" };
            ui.label(arrow);
        });
    });
    
    action
}
