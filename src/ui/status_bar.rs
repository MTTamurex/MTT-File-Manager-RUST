//! Status bar rendering for the file manager.
//!
//! This module contains the rendering logic for the application status bar.

use crate::domain::file_entry::{FoldersPosition, SortMode, ViewMode};
use eframe::egui;
use lru::LruCache;
use std::path::PathBuf;

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
    folders_position: &mut FoldersPosition,
    texture_cache: &LruCache<PathBuf, egui::TextureHandle>,
) -> StatusBarAction {
    let mut action = StatusBarAction::None;

    ui.horizontal(|ui| {
        // === LEFT SIDE: Item count and loading status ===
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

        ui.separator();

        // === CENTER: View mode and thumbnail size ===
        ui.label("Modo:");
        if ui
            .selectable_label(*view_mode == ViewMode::Grid, "Grade")
            .clicked()
        {
            *view_mode = ViewMode::Grid;
            action = StatusBarAction::ViewModeChanged;
        }
        if ui
            .selectable_label(*view_mode == ViewMode::List, "Lista")
            .clicked()
        {
            *view_mode = ViewMode::List;
            action = StatusBarAction::ViewModeChanged;
        }

        ui.separator();

        ui.label("Tamanho:");
        ui.add(egui::Slider::new(thumbnail_size, 64.0..=256.0).show_value(false));

        ui.separator();

        // === CENTER-RIGHT: Sort controls ===
        ui.label("Ordenar:");

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

        ui.separator();

        ui.label("Pastas:");
        if ui
            .selectable_label(*folders_position == FoldersPosition::First, "Início")
            .on_hover_text("Pastas sempre no topo")
            .clicked()
        {
            *folders_position = FoldersPosition::First;
            action = StatusBarAction::SortChanged;
        }
        if ui
            .selectable_label(*folders_position == FoldersPosition::Last, "Fim")
            .on_hover_text("Pastas no final da lista")
            .clicked()
        {
            *folders_position = FoldersPosition::Last;
            action = StatusBarAction::SortChanged;
        }
        if ui
            .selectable_label(*folders_position == FoldersPosition::Mixed, "Misto")
            .on_hover_text("Pastas misturadas com arquivos")
            .clicked()
        {
            *folders_position = FoldersPosition::Mixed;
            action = StatusBarAction::SortChanged;
        }

        // === RIGHT SIDE: System info (push to right with available space) ===
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // RAM usage (appears rightmost)
            if let Some(ram_usage) = get_ram_usage() {
                ui.label(format!("RAM: {}", format_size(ram_usage)));
            }

            // VRAM estimation
            let vram_usage: usize = texture_cache
                .iter()
                .map(|(_, tex)| {
                    let size = tex.size();
                    size[0] as usize * size[1] as usize * 4 // RGBA = 4 bytes per pixel
                })
                .sum();

            ui.label(format!(
                "VRAM: {:.1} MB",
                vram_usage as f64 / 1024.0 / 1024.0
            ));
        });
    });

    action
}

/// Gets the current process RAM usage (RSS/Working Set).
fn get_ram_usage() -> Option<u64> {
    use windows::{
        Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Win32::System::Threading::GetCurrentProcess,
    };

    unsafe {
        let mut counters = PROCESS_MEMORY_COUNTERS::default();
        if K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
        .as_bool()
        {
            Some(counters.WorkingSetSize as u64)
        } else {
            None
        }
    }
}

/// Formats size in bytes to human readable string
fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];

    if bytes == 0 {
        return "0 B".to_string();
    }

    let base = 1024_f64;
    let bytes_f64 = bytes as f64;
    let exponent = (bytes_f64.log10() / base.log10()).floor() as i32;
    let unit_index = exponent.min(5).max(0) as usize;
    let divisor = base.powi(exponent);

    format!("{:.1} {}", bytes_f64 / divisor, UNITS[unit_index])
}
