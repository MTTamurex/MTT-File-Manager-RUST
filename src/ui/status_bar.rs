//! Status bar rendering for the file manager.
//!
//! This module contains the rendering logic for the application status bar.

use crate::domain::file_entry::{FoldersPosition, SortMode, ViewMode};
use crate::ui::theme;
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
    /// Open virtual drive settings
    OpenVirtualDriveSettings,
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
    sort_mode: &mut SortMode,
    sort_descending: &mut bool,
    folders_position: &mut FoldersPosition,
    texture_cache: &LruCache<PathBuf, egui::TextureHandle>,
    frame_time_avg_ms: f32,
    frame_time_peak_ms: f32,
    fps_avg: f32,
    upload_budget_ms: f32,
    is_computer_view: bool,
) -> StatusBarAction {
    let mut action = StatusBarAction::None;

    ui.scope(|ui| {
        let hover_color = if ui.visuals().dark_mode {
            theme::color_dark_hover()
        } else {
            theme::color_hover()
        };
        ui.visuals_mut().override_text_color = Some(egui::Color32::BLACK);
        ui.visuals_mut().widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
        ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
        ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
        ui.visuals_mut().widgets.inactive.fg_stroke = egui::Stroke::NONE;
        ui.visuals_mut().widgets.hovered.bg_fill = hover_color;
        ui.visuals_mut().widgets.hovered.weak_bg_fill = hover_color;
        ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;
        ui.visuals_mut().widgets.hovered.fg_stroke = egui::Stroke::NONE;
        ui.visuals_mut().widgets.active.bg_fill = hover_color;
        ui.visuals_mut().widgets.active.weak_bg_fill = hover_color;
        ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::NONE;
        ui.visuals_mut().widgets.active.fg_stroke = egui::Stroke::NONE;

        ui.horizontal(|ui| {
        // === LEFTMOST: Virtual drive settings button ===
        if ui
            .button(egui::RichText::new("⚙").color(egui::Color32::BLACK))
            .on_hover_text("Configurar otimização de drives virtuais")
            .clicked()
        {
            action = StatusBarAction::OpenVirtualDriveSettings;
        }

        ui.separator();

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

        // === CENTER: View mode ===
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

        // === CENTER-RIGHT: Sort controls ===
        ui.label("Ordenar:");

        // Opções de ordenação diferentes para Computer View
        let sort_modes: Vec<(SortMode, &str)> = if is_computer_view {
            vec![
                (SortMode::Name, "Nome"),
                (SortMode::DriveTotalSpace, "Espaço Total"),
                (SortMode::DriveFreeSpace, "Espaço Livre"),
            ]
        } else {
            vec![
                (SortMode::Name, "Nome"),
                (SortMode::Date, "Data"),
                (SortMode::Size, "Tamanho"),
            ]
        };

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
            if upload_budget_ms > 0.0 {
                ui.label(format!("Upload: {:.0} ms", upload_budget_ms));
            }
            if fps_avg > 0.0 {
                ui.label(format!("FPS: {:.0}", fps_avg));
            }
            if frame_time_avg_ms > 0.0 {
                if frame_time_peak_ms > 0.0 {
                    ui.label(format!("Frame: {:.1} ms ({:.1} ms)", frame_time_avg_ms, frame_time_peak_ms));
                } else {
                    ui.label(format!("Frame: {:.1} ms", frame_time_avg_ms));
                }
            }
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
