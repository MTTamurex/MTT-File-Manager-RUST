// Three 26-point buttons, explicit spaces (2 + 2 + 4), and six item gaps.
const BASIC_FIXED_WIDTH: f32 = 3.0 * 26.0 + 8.0;
const VOLUME_SLIDER_WIDTH: f32 = 70.0;

pub(super) fn docked_volume_slider_width(width: f32, spacing: f32) -> f32 {
    (width - BASIC_FIXED_WIDTH - 6.0 * spacing).clamp(0.0, VOLUME_SLIDER_WIDTH)
}

pub(super) fn docked_controls_need_time_row(width: f32, spacing: f32, time_width: f32) -> bool {
    width < BASIC_FIXED_WIDTH + VOLUME_SLIDER_WIDTH + 10.0 + 8.0 * spacing + time_width
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimum_panel_width_keeps_controls_inside_viewport() {
        for width in [190.0, 200.0, 210.0, 226.0] {
            let spacing = 8.0;
            let slider = docked_volume_slider_width(width, spacing);
            assert!(BASIC_FIXED_WIDTH + 6.0 * spacing + slider <= width);
            assert!(docked_controls_need_time_row(width, spacing, 100.0));
        }
    }

    #[test]
    fn wide_panel_retains_single_row_and_full_volume_slider() {
        assert_eq!(docked_volume_slider_width(400.0, 8.0), 70.0);
        assert!(!docked_controls_need_time_row(400.0, 8.0, 100.0));
    }

    #[test]
    fn long_time_display_is_included_in_row_budget() {
        assert!(!docked_controls_need_time_row(350.0, 8.0, 100.0));
        assert!(docked_controls_need_time_row(350.0, 8.0, 150.0));
    }

    #[test]
    fn rendered_controls_do_not_expand_narrow_layout_at_different_scales() {
        use crate::ui::components::MediaPreview;
        use crate::ui::svg_icons::SvgIconManager;
        use eframe::egui;

        for scale in [1.0, 1.25, 1.5, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(scale);
            let mut icons = SvgIconManager::new();
            let mut preview = MediaPreview::Error(String::new());
            for width in [190.0, 210.0, 226.0, 400.0, 210.0] {
                let _ = ctx.run_ui(Default::default(), |ui| {
                    let rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(width, 300.0));
                    let response = ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                        super::super::draw_docked_controls(
                            ui,
                            &mut preview,
                            width,
                            &mut icons,
                            false,
                            3600.0,
                            36000.0,
                            1.0,
                            false,
                        );
                    });
                    assert!(
                        response.response.rect.width() <= width + 0.1,
                        "controls expanded at width {width}, scale {scale}: {:?}",
                        response.response.rect
                    );
                });
            }
        }
    }
}
