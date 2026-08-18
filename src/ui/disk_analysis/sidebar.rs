//! Left sidebar of the disk analyzer: NTFS drives, volume summary, stats.

use crate::app::disk_analysis_state::DiskAnalysisState;
use crate::infrastructure::windows::formatting::format_size;
use eframe::egui;
use rust_i18n::t;

pub fn render_sidebar(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    ui.add_space(6.0);
    section_header(ui, &t!("disk_analysis.drives"));
    render_drives(state, ui);

    ui.add_space(10.0);
    ui.separator();
    ui.add_space(6.0);
    section_header(ui, &t!("disk_analysis.summary"));
    render_summary(state, ui);
}

/// Volume summary: file/folder counts and used-vs-total space (moved here
/// from the old status bar / top bar).
fn render_summary(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    if let Some(drive) = state
        .drives
        .iter()
        .find(|d| Some(d.letter) == state.drive_letter)
    {
        let used = drive.total_space.saturating_sub(drive.free_space);
        ui.label(
            egui::RichText::new(
                t!(
                    "disk_analysis.used_of",
                    used = format_size(used),
                    total = format_size(drive.total_space)
                )
                .to_string(),
            )
            .strong(),
        );
        ui.add_space(2.0);
    }
    if let Some(model) = state.model.clone() {
        ui.label(
            egui::RichText::new(
                t!("disk_analysis.files_count", count = model.total_files).to_string(),
            )
            .color(ui.visuals().weak_text_color()),
        );
        ui.label(
            egui::RichText::new(
                t!("disk_analysis.folders_count", count = model.total_folders).to_string(),
            )
            .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(t!("disk_analysis.deepest_path").to_string())
                    .color(ui.visuals().weak_text_color()),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(model.deepest_path.to_string());
            });
        });
    }
}

fn section_header(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .size(11.0)
            .color(ui.visuals().weak_text_color()),
    );
    ui.add_space(4.0);
}

fn render_drives(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    let active_letter = state.drive_letter;
    let drives = state.drives.clone();

    for drive in drives {
        if !drive.file_system.eq_ignore_ascii_case("NTFS") {
            continue;
        }
        let used = drive.total_space.saturating_sub(drive.free_space);
        let fraction = if drive.total_space > 0 {
            used as f32 / drive.total_space as f32
        } else {
            0.0
        };
        let is_active = active_letter == Some(drive.letter);

        let response = ui.allocate_response(
            egui::vec2(ui.available_width(), 52.0),
            egui::Sense::click(),
        );
        let rect = response.rect;
        // Border-only feedback, matching the main app's GRID mode:
        // solid accent outline when selected, light accent outline on hover.
        let accent = crate::ui::theme::COLOR_ACCENT;
        if is_active {
            let stroke_width = if response.hovered() { 2.5 } else { 2.0 };
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                6.0,
                egui::Stroke::new(stroke_width, accent),
                egui::StrokeKind::Inside,
            );
        } else if response.hovered() {
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                6.0,
                egui::Stroke::new(1.0, accent.gamma_multiply(0.35)),
                egui::StrokeKind::Inside,
            );
        }

        let text_rect = rect.shrink2(egui::vec2(8.0, 6.0));
        ui.painter().text(
            text_rect.left_top(),
            egui::Align2::LEFT_TOP,
            format!("{}:  {}", drive.letter, drive.label),
            egui::FontId::proportional(13.0),
            ui.visuals().text_color(),
        );
        ui.painter().text(
            text_rect.right_top(),
            egui::Align2::RIGHT_TOP,
            format!("{:.0}%", fraction * 100.0),
            egui::FontId::proportional(12.0),
            ui.visuals().text_color(),
        );
        let bar_rect = egui::Rect::from_min_size(
            egui::pos2(text_rect.min.x, text_rect.min.y + 20.0),
            egui::vec2(text_rect.width(), 4.0),
        );
        ui.painter().rect_filled(
            bar_rect,
            2.0,
            crate::ui::theme::drive_usage_background_color(ui.visuals().dark_mode),
        );
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                bar_rect.min,
                egui::vec2(bar_rect.width() * fraction.clamp(0.0, 1.0), bar_rect.height()),
            ),
            2.0,
            crate::ui::theme::drive_usage_color(fraction),
        );
        ui.painter().text(
            egui::pos2(text_rect.min.x, bar_rect.max.y + 3.0),
            egui::Align2::LEFT_TOP,
            t!(
                "disk_analysis.used_of",
                used = format_size(used),
                total = format_size(drive.total_space)
            )
            .to_string(),
            egui::FontId::proportional(11.0),
            ui.visuals().weak_text_color(),
        );

        if response.clicked() && !is_active {
            state.request(drive.letter);
            ui.ctx().request_repaint();
        }
        ui.add_space(2.0);
    }
}
