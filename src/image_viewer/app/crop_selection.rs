use crate::image_viewer::crop::NormalizedCrop;
use eframe::egui;

pub(super) fn normalized_position(position: egui::Pos2, rect: egui::Rect) -> [f32; 2] {
    [
        ((position.x - rect.left()) / rect.width().max(f32::EPSILON)).clamp(0.0, 1.0),
        ((position.y - rect.top()) / rect.height().max(f32::EPSILON)).clamp(0.0, 1.0),
    ]
}

pub(super) fn drag_origin(position: egui::Pos2, total_delta: egui::Vec2) -> egui::Pos2 {
    position - total_delta
}

pub(super) fn resize_anchor(
    selection: Option<NormalizedCrop>,
    pointer: [f32; 2],
    image_rect: egui::Rect,
) -> Option<[f32; 2]> {
    let selection = selection?;
    let threshold_x = 12.0 / image_rect.width().max(1.0);
    let threshold_y = 12.0 / image_rect.height().max(1.0);
    let corners = [
        (
            [selection.min_x, selection.min_y],
            [selection.max_x, selection.max_y],
        ),
        (
            [selection.max_x, selection.min_y],
            [selection.min_x, selection.max_y],
        ),
        (
            [selection.max_x, selection.max_y],
            [selection.min_x, selection.min_y],
        ),
        (
            [selection.min_x, selection.max_y],
            [selection.max_x, selection.min_y],
        ),
    ];
    corners
        .into_iter()
        .find(|(corner, _)| {
            (pointer[0] - corner[0]).abs() <= threshold_x
                && (pointer[1] - corner[1]).abs() <= threshold_y
        })
        .map(|(_, opposite)| opposite)
}

pub(super) fn paint_crop_overlay(
    painter: &egui::Painter,
    image_rect: egui::Rect,
    crop: NormalizedCrop,
) {
    let selection = egui::Rect::from_min_max(
        egui::pos2(
            image_rect.left() + crop.min_x * image_rect.width(),
            image_rect.top() + crop.min_y * image_rect.height(),
        ),
        egui::pos2(
            image_rect.left() + crop.max_x * image_rect.width(),
            image_rect.top() + crop.max_y * image_rect.height(),
        ),
    );
    let shade = egui::Color32::from_black_alpha(150);
    for rect in [
        egui::Rect::from_min_max(
            image_rect.min,
            egui::pos2(image_rect.max.x, selection.min.y),
        ),
        egui::Rect::from_min_max(
            egui::pos2(image_rect.min.x, selection.max.y),
            image_rect.max,
        ),
        egui::Rect::from_min_max(
            egui::pos2(image_rect.min.x, selection.min.y),
            egui::pos2(selection.min.x, selection.max.y),
        ),
        egui::Rect::from_min_max(
            egui::pos2(selection.max.x, selection.min.y),
            egui::pos2(image_rect.max.x, selection.max.y),
        ),
    ] {
        painter.rect_filled(rect, 0.0, shade);
    }
    painter.rect_stroke(
        selection,
        0.0,
        egui::Stroke::new(2.0, egui::Color32::WHITE),
        egui::StrokeKind::Inside,
    );
    for corner in [
        selection.left_top(),
        selection.right_top(),
        selection.right_bottom(),
        selection.left_bottom(),
    ] {
        painter.circle_filled(corner, 5.0, egui::Color32::WHITE);
        painter.circle_stroke(corner, 5.0, egui::Stroke::new(1.0, egui::Color32::BLACK));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_position_clamps_outside_image() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(100.0, 50.0));

        assert_eq!(
            normalized_position(egui::pos2(-5.0, 100.0), rect),
            [0.0, 1.0]
        );
    }

    #[test]
    fn corner_resize_uses_opposite_corner_as_anchor() {
        let selection = NormalizedCrop::new([0.2, 0.3], [0.8, 0.9]);
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 100.0));

        assert_eq!(
            resize_anchor(selection, [0.21, 0.31], rect),
            Some([0.8, 0.9])
        );
    }

    #[test]
    fn drag_origin_restores_pointer_press_position() {
        assert_eq!(
            drag_origin(egui::pos2(42.0, 31.0), egui::vec2(12.0, -9.0)),
            egui::pos2(30.0, 40.0)
        );
    }
}
