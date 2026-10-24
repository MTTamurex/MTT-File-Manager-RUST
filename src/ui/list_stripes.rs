//! Optional file-list backgrounds, shared by all non-grid views.
use eframe::egui::{self, Color32};

pub(crate) const PREFERENCE_KEY: &str = "list_stripes_enabled";

pub(crate) fn from_preference(value: Option<&str>) -> bool {
    value == Some("true")
}

pub(crate) fn remember(ctx: &egui::Context, enabled: bool) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(PREFERENCE_KEY), enabled));
    ctx.request_repaint();
}

fn row_color(ctx: &egui::Context, row: usize) -> Option<Color32> {
    let enabled = ctx
        .data(|data| data.get_temp::<bool>(egui::Id::new(PREFERENCE_KEY)))
        .unwrap_or(false);
    if !enabled || row.is_multiple_of(2) {
        return None;
    }
    // A translucent theme foreground keeps custom palettes' hue intact.
    Some(crate::ui::theme::text_color(ctx).gamma_multiply(0.05))
}

pub(crate) fn paint(ui: &egui::Ui, rect: egui::Rect, row: usize) {
    if ui.is_rect_visible(rect) {
        if let Some(color) = row_color(ui.ctx(), row) {
            ui.painter().rect_filled(rect, 4.0, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::navigation_state::ThemeMode;

    #[test]
    fn disabled_by_default_and_after_toggle_off() {
        let ctx = egui::Context::default();
        assert_eq!(row_color(&ctx, 1), None);
        remember(&ctx, true);
        assert!(row_color(&ctx, 1).is_some());
        remember(&ctx, false);
        assert_eq!(row_color(&ctx, 1), None);
    }

    #[test]
    fn painting_only_adds_a_background_for_visible_alternating_rows() {
        let ctx = egui::Context::default();
        remember(&ctx, true);
        let color = row_color(&ctx, 1).unwrap();
        for (enabled, expected) in [(true, 1), (false, 0)] {
            remember(&ctx, enabled);
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_clip_rect(egui::Rect::from_min_size(
                    ui.min_rect().min,
                    egui::vec2(200.0, 200.0),
                ));
                let rect = egui::Rect::from_min_size(ui.min_rect().min, egui::vec2(100.0, 24.0));
                paint(ui, rect, 0);
                paint(ui, rect.translate(egui::vec2(0.0, 24.0)), 1);
                paint(ui, rect.translate(egui::vec2(0.0, 48.0)), 2);
                paint(ui, rect.translate(egui::vec2(0.0, 10000.0)), 3);
            });
            let stripe_count = output.shapes.iter().filter(|shape| {
                matches!(&shape.shape, egui::epaint::Shape::Rect(rect) if rect.fill == color)
            }).count();
            assert_eq!(stripe_count, expected);
        }
    }

    #[test]
    fn alternation_is_stable_at_any_scroll_position_and_theme() {
        let ctx = egui::Context::default();
        remember(&ctx, true);
        for mode in [
            ThemeMode::Light,
            ThemeMode::Dark,
            ThemeMode::DarkDeep,
            ThemeMode::GitHubDark,
            ThemeMode::System,
        ] {
            crate::ui::theme::apply_theme_visuals(&ctx, mode);
            for row in [0, 1, 2, 101, 102, 10001] {
                assert_eq!(row_color(&ctx, row).is_some(), row % 2 == 1);
            }
            let color = row_color(&ctx, 1).unwrap();
            assert!(color.a() > 0 && color.a() < 32);
        }
        crate::ui::theme::apply_theme_visuals(&ctx, ThemeMode::Light);
        let light = row_color(&ctx, 1).unwrap();
        crate::ui::theme::apply_theme_visuals(&ctx, ThemeMode::Dark);
        assert_ne!(light, row_color(&ctx, 1).unwrap());
    }
}
