use crate::ui::components::MediaPreview;
use eframe::egui;

fn seek_slider(ui: &mut egui::Ui, position: &mut f64, duration: f64) -> egui::Response {
    let ready = duration.is_finite() && duration > 0.0;
    let end = if ready { duration } else { 1.0 };
    // Sanitize before constructing the widget: egui clamping is not a user seek.
    *position = if ready && position.is_finite() {
        position.clamp(0.0, end)
    } else {
        0.0
    };
    ui.add_enabled(
        ready,
        egui::Slider::new(position, 0.0..=end)
            .show_value(false)
            .trailing_fill(true),
    )
}

pub(super) fn draw_seek_bar(
    ui: &mut egui::Ui,
    preview: &mut MediaPreview,
    full_width: f32,
    current_time: f64,
    duration: f64,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().slider_width = full_width;
        ui.visuals_mut().selection.bg_fill = crate::ui::theme::accent_color(ui.ctx());

        let mut seek_value = current_time;
        let response = seek_slider(ui, &mut seek_value, duration);
        if response.enabled() && response.changed() {
            preview.seek(seek_value);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        ctx: &egui::Context,
        value: &mut f64,
        duration: f64,
        input: egui::RawInput,
    ) -> egui::Response {
        let mut response = None;
        let _ = ctx.run_ui(input, |ui| {
            response = Some(seek_slider(ui, value, duration));
        });
        response.expect("slider rendered")
    }

    fn render(position: f64, duration: f64) -> (f64, bool, bool) {
        let ctx = egui::Context::default();
        let mut value = position;
        let response = frame(&ctx, &mut value, duration, Default::default());
        (value, response.enabled(), response.changed())
    }

    #[test]
    fn position_before_duration_keeps_the_bar_at_start_without_seeking() {
        assert_eq!(render(0.75, 0.0), (0.0, false, false));
        assert_eq!(render(0.75, 120.0), (0.75, true, false));
    }

    #[test]
    fn invalid_duration_disables_seeking() {
        for duration in [-1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(render(0.75, duration), (0.0, false, false));
        }
    }

    #[test]
    fn clamping_playback_values_does_not_emit_a_user_seek() {
        assert_eq!(render(121.0, 120.0), (120.0, true, false));
        assert_eq!(render(-1.0, 120.0), (0.0, true, false));
        assert_eq!(render(f64::NAN, 120.0), (0.0, true, false));
    }

    #[test]
    fn very_short_videos_use_their_actual_duration() {
        assert_eq!(render(0.025, 0.05), (0.025, true, false));
    }

    #[test]
    fn user_click_seeks_only_after_duration_is_known() {
        for duration in [0.0, 120.0] {
            let ctx = egui::Context::default();
            let mut value = 0.0;
            let response = frame(&ctx, &mut value, duration, Default::default());
            let pos = response.rect.center();
            let input = egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            };
            let response = frame(&ctx, &mut value, duration, input);
            if duration == 0.0 {
                assert_eq!(value, 0.0);
                assert!(!response.changed());
                assert!(!response.enabled());
            } else {
                assert!(response.changed());
                assert!(value > 0.0 && value < duration);
            }
        }
    }
}
