use super::*;
use crate::app::shortcuts::{ShortcutAction, ShortcutBinding, ShortcutBindings};

#[test]
fn select_all_hover_tooltip_displays_the_current_shortcut() {
    for binding in [
        ShortcutBinding::ctrl(egui::Key::A),
        ShortcutBinding::ctrl_shift(egui::Key::K),
    ] {
        for item_count in [0, 5] {
            let ctx = egui::Context::default();
            ctx.global_style_mut(|style| {
                style.animation_time = 0.0;
                style.interaction.tooltip_delay = 0.0;
                style.interaction.show_tooltips_only_when_still = false;
            });
            let mut shortcuts = ShortcutBindings::default();
            shortcuts.set(ShortcutAction::SelectAll, binding);
            let mut svg = SvgIconManager::new();
            let bar_rect =
                egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(800.0, 32.0));
            let pointer = bar_rect.center() - egui::vec2(BUTTON_SIZE.x + BUTTON_SPACING, 0.0);
            let expected = format!(
                "{} ({})",
                t!("status_bar.select_all"),
                shortcuts.label(ShortcutAction::SelectAll)
            );
            let mut found = false;
            for frame in 0..6 {
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1000.0, 300.0),
                        )),
                        time: Some(frame as f64),
                        events: vec![egui::Event::PointerMoved(pointer)],
                        ..Default::default()
                    },
                    |ui| {
                        assert!(
                            render(ui, bar_rect, &mut svg, item_count, false, &shortcuts).is_none()
                        );
                    },
                );
                found = output.shapes.iter().any(|shape| {
                    matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == expected)
                });
            }
            assert!(found, "hovering Select all must show {expected}");
        }
    }
}
