use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke};
use mtt_search_protocol::DriveHealthState;

use crate::domain::file_entry::DriveInfo;

pub(crate) fn paint_health_badge(painter: &Painter, icon_rect: Rect, drive_info: &DriveInfo) {
    let Some(style) = badge_style(
        drive_info
            .health
            .as_ref()
            .map(|snapshot| &snapshot.health_state),
    ) else {
        return;
    };

    let size = (icon_rect.width().min(icon_rect.height()) * 0.4).clamp(9.0, 18.0);
    let bottom = icon_rect.bottom() + 1.0;
    let center_x = icon_rect.right() - size * 0.45;
    paint_health_triangle(painter, center_x, bottom, size, style);
}

pub(crate) fn paint_health_badge_in_rect(painter: &Painter, rect: Rect, state: &DriveHealthState) {
    let Some(style) = badge_style(Some(state)) else {
        return;
    };
    let size = rect.width().min(rect.height());
    paint_health_triangle(painter, rect.center().x, rect.bottom(), size, style);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BadgeStyle {
    fill: Color32,
    outline: Color32,
    mark: Color32,
}

fn paint_health_triangle(
    painter: &Painter,
    center_x: f32,
    bottom: f32,
    size: f32,
    style: BadgeStyle,
) {
    let points = vec![
        Pos2::new(center_x, bottom - size),
        Pos2::new(center_x - size * 0.52, bottom),
        Pos2::new(center_x + size * 0.52, bottom),
    ];

    painter.add(Shape::convex_polygon(
        points,
        style.fill,
        Stroke::new((size * 0.1).max(1.0), style.outline),
    ));

    painter.line_segment(
        [
            Pos2::new(center_x, bottom - size * 0.68),
            Pos2::new(center_x, bottom - size * 0.35),
        ],
        Stroke::new((size * 0.12).max(1.0), style.mark),
    );
    painter.circle_filled(
        Pos2::new(center_x, bottom - size * 0.2),
        (size * 0.07).max(0.75),
        style.mark,
    );
}

fn badge_style(state: Option<&DriveHealthState>) -> Option<BadgeStyle> {
    match state {
        Some(DriveHealthState::Warning) => Some(BadgeStyle {
            fill: Color32::from_rgb(255, 193, 7),
            outline: Color32::from_rgb(72, 54, 0),
            mark: Color32::from_rgb(45, 34, 0),
        }),
        Some(DriveHealthState::Critical) => Some(BadgeStyle {
            fill: Color32::from_rgb(220, 53, 69),
            outline: Color32::from_rgb(84, 12, 23),
            mark: Color32::WHITE,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::badge_style;
    use mtt_search_protocol::DriveHealthState;

    #[test]
    fn badge_is_shown_only_for_warning_and_critical_health() {
        assert!(badge_style(Some(&DriveHealthState::Warning)).is_some());
        assert!(badge_style(Some(&DriveHealthState::Critical)).is_some());
        assert!(badge_style(Some(&DriveHealthState::Good)).is_none());
        assert!(badge_style(Some(&DriveHealthState::Unknown)).is_none());
        assert!(badge_style(None).is_none());
    }

    #[test]
    fn critical_badge_is_visually_distinct_from_warning() {
        assert_ne!(
            badge_style(Some(&DriveHealthState::Warning)),
            badge_style(Some(&DriveHealthState::Critical))
        );
    }
}
