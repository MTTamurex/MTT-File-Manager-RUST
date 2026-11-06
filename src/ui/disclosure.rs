//! Shared filled-triangle indicators for expand/collapse controls.
use eframe::egui;

pub(crate) const SIDEBAR_SIZE: f32 = 6.0;
pub(crate) const SECTION_SIZE: f32 = 8.0;
pub(crate) const GROUP_SIZE: f32 = 10.0;

pub(crate) fn points(center: egui::Pos2, expanded: bool, size: f32) -> [egui::Pos2; 3] {
    let offsets = if expanded {
        [
            egui::vec2(-0.5, -0.4),
            egui::vec2(0.5, -0.4),
            egui::vec2(0.0, 0.5),
        ]
    } else {
        [
            egui::vec2(-0.4, -0.5),
            egui::vec2(0.5, 0.0),
            egui::vec2(-0.4, 0.5),
        ]
    };
    offsets.map(|offset| center + offset * size)
}

pub(crate) fn paint(
    painter: &egui::Painter,
    center: egui::Pos2,
    expanded: bool,
    size: f32,
    color: egui::Color32,
) {
    painter.add(egui::Shape::convex_polygon(
        points(center, expanded, size).to_vec(),
        color,
        egui::Stroke::NONE,
    ));
}

/// Keep egui collapsing headers' rotation animation and existing hit boxes.
pub(crate) fn collapsing_icon(ui: &mut egui::Ui, openness: f32, response: &egui::Response) {
    let center = response.rect.center();
    let size = SECTION_SIZE
        .min(response.rect.width())
        .min(response.rect.height());
    let mut vertices = points(center, false, size);
    if openness >= 1.0 {
        vertices = points(center, true, size);
    } else if openness > 0.0 {
        let rotation = egui::emath::Rot2::from_angle(openness * std::f32::consts::FRAC_PI_2);
        vertices = vertices.map(|point| center + rotation * (point - center));
    }
    ui.painter().add(egui::Shape::convex_polygon(
        vertices.to_vec(),
        ui.style().interact(response).fg_stroke.color,
        egui::Stroke::NONE,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animated_headers_keep_shared_endpoints_and_a_visible_intermediate_triangle() {
        for openness in [0.0, 0.5, 1.0] {
            let ctx = egui::Context::default();
            let mut center = egui::Pos2::ZERO;
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let response = ui.allocate_response(egui::vec2(18.0, 18.0), egui::Sense::click());
                center = response.rect.center();
                collapsing_icon(ui, openness, &response);
            });
            let triangle = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Path(path) if path.closed && path.points.len() == 3 => Some(path),
                    _ => None,
                })
                .expect("animated header paints a triangle");
            if openness == 0.0 || openness == 1.0 {
                assert_eq!(
                    triangle.points,
                    points(center, openness == 1.0, SECTION_SIZE)
                );
            } else {
                assert!(egui::Rect::from_points(&triangle.points).area() > 0.0);
            }
        }
    }

    #[test]
    fn triangles_have_consistent_direction_and_scale_for_every_context() {
        let center = egui::pos2(30.0, 30.0);
        for size in [SIDEBAR_SIZE, SECTION_SIZE, GROUP_SIZE] {
            let right = points(center, false, size);
            assert!(right[1].x > right[0].x && right[1].x > right[2].x);
            assert_eq!(right[1].y, center.y);
            let down = points(center, true, size);
            assert!(down[2].y > down[0].y && down[2].y > down[1].y);
            assert_eq!(down[2].x, center.x);
            for vertices in [right, down] {
                let bounds = egui::Rect::from_points(&vertices);
                assert!((bounds.width().max(bounds.height()) - size).abs() < 0.01);
            }
        }
        assert!(GROUP_SIZE > SIDEBAR_SIZE);
    }
}
