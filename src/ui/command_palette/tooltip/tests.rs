use super::*;

#[test]
fn tooltips_open_beside_the_pointer_for_enabled_and_disabled_commands() {
    for enabled in [true, false] {
        for pointer in [egui::pos2(100.0, 120.0), egui::pos2(1100.0, 660.0)] {
            let ctx = egui::Context::default();
            ctx.global_style_mut(|style| {
                style.animation_time = 0.0;
                style.interaction.tooltip_delay = 0.0;
                style.interaction.show_tooltips_only_when_still = false;
            });
            let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 700.0));
            let mut tooltip_rect = None;
            for frame in 0..6 {
                let _ = ctx.run_ui(egui::RawInput {
                    screen_rect: Some(viewport),
                    time: Some(frame as f64),
                    events: vec![egui::Event::PointerMoved(pointer)],
                    ..Default::default()
                }, |ui| {
                    let row_rect = egui::Rect::from_min_size(
                        egui::pos2(16.0, pointer.y - 8.0), egui::vec2(1168.0, 32.0),
                    );
                    let response = ui.add_enabled_ui(enabled, |ui| {
                        ui.interact(row_rect, egui::Id::new("command"), egui::Sense::click())
                    }).inner;
                    tooltip_rect = show(&response, "Command description explaining the action and its keyboard and mouse shortcuts.");
                });
            }
            let rect = tooltip_rect.expect("hovering must show the command tooltip");
            assert!(!rect.contains(pointer), "{rect:?}");
            assert!(viewport.contains_rect(rect), "{rect:?}");
            if pointer.x < 1000.0 {
                assert!(rect.left() >= pointer.x + POINTER_GAP - 1.0, "{rect:?}");
                assert!(
                    (rect.top() - pointer.y).abs() < 2.0,
                    "do not anchor below the row: {rect:?}"
                );
            } else {
                assert!(rect.right() <= pointer.x - POINTER_GAP + 1.0, "{rect:?}");
            }
        }
    }
}
