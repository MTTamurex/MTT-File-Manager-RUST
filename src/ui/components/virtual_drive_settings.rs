//! Virtual drive settings modal for configuring SSD/HDD optimization

use crate::infrastructure::io_priority;
use crate::infrastructure::virtual_drive_config::{
    detect_virtual_drives as detect_virtual_drives_from_system, get_all_overrides,
    remove_drive_override, set_drive_override, DiskTypeOverride,
};
use eframe::egui;
use rust_i18n::t;

/// Info about a detected virtual drive
#[derive(Clone)]
struct VirtualDriveInfo {
    letter: char,
    label: String,
    file_system: String,
    current_override: Option<DiskTypeOverride>,
}

pub fn render_virtual_drive_settings_section(ui: &mut egui::Ui) {
    ui.heading(t!("settings.virtual_drives"));
    ui.add_space(8.0);
    ui.label(t!("vdrive_settings.description"));
    ui.label(t!("vdrive_settings.ssd_desc"));
    ui.label(t!("vdrive_settings.hdd_desc"));
    ui.add_space(16.0);

    let virtual_drives = load_virtual_drives();

    if virtual_drives.is_empty() {
        ui.colored_label(
            egui::Color32::from_rgb(200, 200, 0),
            t!("vdrive_settings.no_drives"),
        );
        ui.add_space(8.0);
        ui.label(t!("vdrive_settings.typical_drives"));
        ui.label("• Cryptomator (CryptoFS)");
        ui.label("• Dokan");
        ui.label("• WinFsp");
    } else {
        egui::Grid::new("virtual_drives_grid")
            .striped(true)
            .min_col_width(60.0)
            .show(ui, |ui| {
                ui.strong(t!("vdrive_settings.col_drive"));
                ui.strong(t!("vdrive_settings.col_label"));
                ui.strong(t!("vdrive_settings.col_system"));
                ui.strong(t!("vdrive_settings.col_optimization"));
                ui.strong(t!("vdrive_settings.col_actions"));
                ui.end_row();

                for drive_info in &virtual_drives {
                    render_drive_row(ui, drive_info);
                }
            });
    }

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(8.0);
    ui.label(t!("vdrive_settings.config_info"));
}

/// Detect all virtual drives in the system
fn load_virtual_drives() -> Vec<VirtualDriveInfo> {
    let mut virtual_drives = Vec::new();
    let overrides = get_all_overrides();

    for drive in detect_virtual_drives_from_system() {
        virtual_drives.push(VirtualDriveInfo {
            letter: drive.letter,
            label: drive.label,
            file_system: drive.file_system,
            current_override: overrides.get(&drive.letter).copied(),
        });
    }

    virtual_drives
}

/// Render a single drive configuration row
fn render_drive_row(ui: &mut egui::Ui, drive_info: &VirtualDriveInfo) {
    // Drive letter
    ui.label(format!("{}:\\", drive_info.letter));

    // Label
    ui.label(&drive_info.label);

    // File system
    ui.label(&drive_info.file_system);

    // Current setting
    let current_type = drive_info.current_override.unwrap_or(DiskTypeOverride::SSD);
    let is_ssd = matches!(current_type, DiskTypeOverride::SSD);

    ui.horizontal(|ui| {
        if ui
            .selectable_label(is_ssd, "SSD")
            .on_hover_text(t!("vdrive_settings.ssd_hint"))
            .clicked()
        {
            if let Err(e) = set_drive_override(drive_info.letter, DiskTypeOverride::SSD) {
                log::error!("[Config] Failed to set SSD override: {}", e);
            } else {
                io_priority::invalidate_drive_cache(drive_info.letter);
            }
        }

        if ui
            .selectable_label(!is_ssd, "HDD")
            .on_hover_text(t!("vdrive_settings.hdd_hint"))
            .clicked()
        {
            if let Err(e) = set_drive_override(drive_info.letter, DiskTypeOverride::HDD) {
                log::error!("[Config] Failed to set HDD override: {}", e);
            } else {
                io_priority::invalidate_drive_cache(drive_info.letter);
            }
        }
    });

    // Actions
    ui.horizontal(|ui| {
        if drive_info.current_override.is_some() {
            if ui
                .button("🗑")
                .on_hover_text(t!("vdrive_settings.remove"))
                .clicked()
            {
                if let Err(e) = remove_drive_override(drive_info.letter) {
                    log::error!("[Config] Failed to remove override: {}", e);
                } else {
                    io_priority::invalidate_drive_cache(drive_info.letter);
                }
            }
        } else {
            ui.label(&*t!("vdrive_settings.default_label"));
        }
    });

    ui.end_row();
}
