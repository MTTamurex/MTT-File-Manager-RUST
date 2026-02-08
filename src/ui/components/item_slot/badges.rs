use super::*;

/// Renders a sync status badge (OneDrive) on the bottom-right corner of the thumbnail
pub(super) fn render_sync_badge(ui: &mut egui::Ui, thumb_rect: egui::Rect, status: SyncStatus) {
    if status == SyncStatus::None {
        return; // No badge for normal files
    }

    let badge_size = 18.0;
    let badge_pos = egui::pos2(
        thumb_rect.right() - badge_size - 2.0,
        thumb_rect.bottom() - badge_size - 2.0,
    );
    let badge_center = badge_pos + egui::vec2(badge_size / 2.0, badge_size / 2.0);
    let badge_radius = badge_size / 2.0;

    let painter = ui.painter();

    match status {
        SyncStatus::CloudOnly => {
            // Blue cloud icon - file needs download
            painter.circle_filled(
                badge_center,
                badge_radius,
                egui::Color32::from_rgb(0, 120, 215),
            );
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "☁",
                egui::FontId::proportional(12.0),
                egui::Color32::WHITE,
            );
        }
        SyncStatus::Syncing => {
            // Blue circular arrows - file is being synced
            painter.circle_filled(
                badge_center,
                badge_radius,
                egui::Color32::from_rgb(0, 120, 215),
            );
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "⟳",
                egui::FontId::proportional(13.0),
                egui::Color32::WHITE,
            );
        }
        SyncStatus::Pinned => {
            // Green solid circle with check - always keep on device
            painter.circle_filled(
                badge_center,
                badge_radius,
                egui::Color32::from_rgb(0, 150, 0),
            );
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "✓",
                egui::FontId::proportional(11.0),
                egui::Color32::WHITE,
            );
        }
        SyncStatus::LocallyAvailable => {
            // White circle with green outline/check - downloaded on demand
            painter.circle_filled(badge_center, badge_radius, egui::Color32::WHITE);
            painter.circle_stroke(
                badge_center,
                badge_radius - 1.0,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(0, 150, 0)),
            );
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "✓",
                egui::FontId::proportional(11.0),
                egui::Color32::from_rgb(0, 150, 0),
            );
        }
        SyncStatus::None => {} // Already handled above
    }
}
