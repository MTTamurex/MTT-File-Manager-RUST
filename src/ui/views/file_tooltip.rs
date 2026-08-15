//! Shared modern tooltip building blocks for file items (list/grid/search).

use crate::domain::file_entry::DriveInfo;
use crate::infrastructure::windows::format_size;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

/// Tooltip title + separator.
pub fn header(ui: &mut egui::Ui, name: &str) {
    let dark_mode = ui.visuals().dark_mode;
    ui.label(
        RichText::new(name)
            .size(13.0)
            .strong()
            .color(theme::text_color(dark_mode)),
    );
    ui.add_space(2.0);
    ui.separator();
    ui.add_space(4.0);
}

/// Centered media preview on a rounded card.
pub fn media_preview(ui: &mut egui::Ui, tex: &egui::TextureHandle) {
    let dark_mode = ui.visuals().dark_mode;
    let tex_size = tex.size_vec2();
    if tex_size.x <= 0.0 || tex_size.y <= 0.0 {
        return;
    }
    let max_w = 280.0_f32;
    let max_h = 180.0_f32;
    let scale = (max_w / tex_size.x).min(max_h / tex_size.y).min(1.0);
    let display_size = egui::vec2(tex_size.x * scale, tex_size.y * scale);

    ui.vertical_centered(|ui| {
        egui::Frame::new()
            .inner_margin(egui::Margin::same(6))
            .corner_radius(6.0)
            .fill(if dark_mode {
                egui::Color32::from_gray(50)
            } else {
                egui::Color32::WHITE
            })
            .stroke(egui::Stroke::new(
                1.0,
                if dark_mode {
                    egui::Color32::from_gray(70)
                } else {
                    egui::Color32::from_gray(224)
                },
            ))
            .show(ui, |ui| {
                ui.add(egui::Image::new(tex).fit_to_exact_size(display_size));
            });
    });
    ui.add_space(4.0);
}

/// Same as [`media_preview`] for an optional texture.
pub fn media_preview_from_option(ui: &mut egui::Ui, tex: Option<&egui::TextureHandle>) {
    if let Some(tex) = tex {
        media_preview(ui, tex);
    }
}

/// Aligned two-column info row: secondary label, primary value.
pub fn info_row(ui: &mut egui::Ui, label: &str, value: &str) {
    let dark_mode = ui.visuals().dark_mode;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(110.0, 18.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(
                    RichText::new(label)
                        .size(12.0)
                        .color(theme::secondary_text_color(dark_mode)),
                );
            },
        );
        ui.label(
            RichText::new(value)
                .size(12.0)
                .color(theme::text_color(dark_mode)),
        );
    });
    ui.add_space(1.0);
}

/// Full drive tooltip: title header + drive metadata rows.
pub fn drive_tooltip_body(ui: &mut egui::Ui, name: &str, drive: &DriveInfo) {
    ui.set_max_width(300.0);
    ui.vertical(|ui| {
        header(ui, name);
        drive_rows(ui, drive);
    });
}

/// Drive metadata rows (shared by list and grid tooltips).
pub fn drive_rows(ui: &mut egui::Ui, drive: &DriveInfo) {
    let file_system = if drive.file_system.is_empty() {
        "NTFS".to_string()
    } else {
        drive.file_system.clone()
    };
    let used_space = drive.total_space.saturating_sub(drive.free_space);

    info_row(
        ui,
        &t!("file_info.type"),
        &format!("{:?}", drive.drive_type),
    );
    info_row(ui, &t!("file_info.used_space"), &format_size(used_space));
    info_row(
        ui,
        &t!("file_info.free_space"),
        &format_size(drive.free_space),
    );
    info_row(
        ui,
        &t!("file_info.total_space"),
        &format_size(drive.total_space),
    );
    info_row(ui, &t!("file_info.filesystem"), &file_system);
}
