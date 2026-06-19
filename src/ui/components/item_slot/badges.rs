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

pub(super) fn render_tag_badge(
    ui: &mut egui::Ui,
    thumb_rect: egui::Rect,
    tag_ids: Option<&[i64]>,
    tag_definitions: &rustc_hash::FxHashMap<i64, crate::domain::file_tag::FileTag>,
) {
    let Some(tag_ids) = tag_ids else {
        return;
    };
    if tag_ids.is_empty() {
        return;
    }

    let mut colors = [egui::Color32::TRANSPARENT; 3];
    let mut color_count = 0usize;
    for color in tag_ids
        .iter()
        .filter_map(|id| tag_definitions.get(id).map(|tag| tag.color.to_color32()))
        .take(3)
    {
        colors[color_count] = color;
        color_count += 1;
    }
    if color_count == 0 {
        return;
    }

    let painter = ui.painter();
    if color_count == 1 {
        let center = egui::pos2(thumb_rect.left() + 8.0, thumb_rect.top() + 8.0);
        painter.circle_filled(center, 4.5, colors[0]);
        painter.circle_stroke(
            center,
            4.5,
            egui::Stroke::new(1.0, egui::Color32::from_black_alpha(80)),
        );
        return;
    }

    let start = egui::pos2(thumb_rect.left() + 6.0, thumb_rect.top() + 7.0);
    for (idx, color) in colors.iter().take(color_count).enumerate() {
        let center = start + egui::vec2(idx as f32 * 7.0, 0.0);
        painter.circle_filled(center, 3.2, *color);
        painter.circle_stroke(
            center,
            3.2,
            egui::Stroke::new(0.8, egui::Color32::from_black_alpha(70)),
        );
    }
}
