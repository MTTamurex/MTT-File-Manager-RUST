//! Helper functions for list view: file type strings, status badges, section headers

use eframe::egui::{self, Color32, FontId, Pos2, RichText, Ui};
use rust_i18n::t;

use crate::domain::file_entry::{FileEntry, SyncStatus};

const SECTION_HEADER_LEFT_PADDING: f32 = 8.0;

/// Renders a section header for grouped views (Computer View)
pub(super) fn render_section_header(ui: &mut Ui, title: &str) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(SECTION_HEADER_LEFT_PADDING);
        ui.label(
            RichText::new(title)
                .size(13.0)
                .color(Color32::from_gray(120))
                .strong(),
        );
    });
    ui.add_space(4.0);
}

/// Returns translated file type string for display.
pub(super) fn get_file_type_string(item: &FileEntry) -> String {
    if let Some(label) = crate::domain::file_entry::archive_type_label(&item.name) {
        return label;
    }
    if item.is_dir {
        return t!("file_types.folder").to_string();
    }

    if let Some(ext) = item.path.extension() {
        return t!(
            "file_info.file_generic",
            ext = ext.to_string_lossy().to_uppercase()
        )
        .to_string();
    }

    t!("file_info.file_unknown").to_string()
}

/// Renders a sync status badge (OneDrive) in the status column
pub(super) fn render_status_badge(ui: &mut egui::Ui, pos: Pos2, status: SyncStatus) {
    if status == SyncStatus::None {
        return; // No badge for normal files
    }

    let badge_size = 16.0;
    let badge_center = pos + egui::vec2(badge_size / 2.0, badge_size / 2.0);
    let badge_radius = badge_size / 2.0;

    let painter = ui.painter();

    match status {
        SyncStatus::CloudOnly => {
            // Blue cloud icon - file needs download
            painter.circle_filled(badge_center, badge_radius, Color32::from_rgb(0, 120, 215));
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "☁",
                FontId::proportional(11.0),
                Color32::WHITE,
            );
        }
        SyncStatus::Syncing => {
            // Blue circular arrows - file is being synced
            painter.circle_filled(badge_center, badge_radius, Color32::from_rgb(0, 120, 215));
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "⟳",
                FontId::proportional(12.0),
                Color32::WHITE,
            );
        }
        SyncStatus::Pinned => {
            // Green solid circle with check - always keep on device
            painter.circle_filled(badge_center, badge_radius, Color32::from_rgb(0, 150, 0));
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "✓",
                FontId::proportional(10.0),
                Color32::WHITE,
            );
        }
        SyncStatus::LocallyAvailable => {
            // White circle with green outline/check - downloaded on demand
            painter.circle_filled(badge_center, badge_radius, Color32::WHITE);
            painter.circle_stroke(
                badge_center,
                badge_radius - 1.0,
                egui::Stroke::new(2.0, Color32::from_rgb(0, 150, 0)),
            );
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "✓",
                FontId::proportional(10.0),
                Color32::from_rgb(0, 150, 0),
            );
        }
        SyncStatus::None => {} // Already handled above
    }
}
