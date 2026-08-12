use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke};
use mtt_search_protocol::DriveHealthState;

use crate::domain::file_entry::DriveInfo;

pub(crate) fn paint_warning_badge(painter: &Painter, icon_rect: Rect, drive_info: &DriveInfo) {
    if !is_warning_state(
        drive_info
            .health
            .as_ref()
            .map(|snapshot| &snapshot.health_state),
    ) {
        return;
    }

    let size = (icon_rect.width().min(icon_rect.height()) * 0.4).clamp(9.0, 18.0);
    let bottom = icon_rect.bottom() + 1.0;
    let center_x = icon_rect.right() - size * 0.45;
    paint_warning_triangle(painter, center_x, bottom, size);
}

pub(crate) fn paint_warning_badge_in_rect(painter: &Painter, rect: Rect) {
    let size = rect.width().min(rect.height());
    paint_warning_triangle(painter, rect.center().x, rect.bottom(), size);
}

fn paint_warning_triangle(painter: &Painter, center_x: f32, bottom: f32, size: f32) {
    let points = vec![
        Pos2::new(center_x, bottom - size),
        Pos2::new(center_x - size * 0.52, bottom),
        Pos2::new(center_x + size * 0.52, bottom),
    ];

    painter.add(Shape::convex_polygon(
        points,
        Color32::from_rgb(255, 193, 7),
        Stroke::new((size * 0.1).max(1.0), Color32::from_rgb(72, 54, 0)),
    ));

    let mark_color = Color32::from_rgb(45, 34, 0);
    painter.line_segment(
        [
            Pos2::new(center_x, bottom - size * 0.68),
            Pos2::new(center_x, bottom - size * 0.35),
        ],
        Stroke::new((size * 0.12).max(1.0), mark_color),
    );
    painter.circle_filled(
        Pos2::new(center_x, bottom - size * 0.2),
        (size * 0.07).max(0.75),
        mark_color,
    );
}

fn is_warning_state(state: Option<&DriveHealthState>) -> bool {
    matches!(state, Some(DriveHealthState::Warning))
}

#[cfg(test)]
mod tests {
    use super::is_warning_state;
    use mtt_search_protocol::DriveHealthState;

    #[test]
    fn warning_badge_is_limited_to_warning_health() {
        assert!(is_warning_state(Some(&DriveHealthState::Warning)));
        assert!(!is_warning_state(Some(&DriveHealthState::Good)));
        assert!(!is_warning_state(Some(&DriveHealthState::Critical)));
        assert!(!is_warning_state(Some(&DriveHealthState::Unknown)));
        assert!(!is_warning_state(None));
    }
}
