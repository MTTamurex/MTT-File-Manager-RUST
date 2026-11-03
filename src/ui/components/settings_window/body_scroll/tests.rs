use super::*;

#[test]
fn small_windows_scroll_both_axes_without_expanding_the_window_or_moving_the_header() {
    for size in [egui::vec2(320.0, 240.0), egui::vec2(520.0, 300.0)] {
        let ctx = egui::Context::default();
        let mut header = egui::Rect::NOTHING;
        let mut offset = egui::Vec2::ZERO;
        let mut viewport = egui::Rect::NOTHING;
        for frame in 0..8 {
            let events = if frame == 4 {
                vec![
                    egui::Event::PointerMoved(viewport.center()),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        phase: egui::TouchPhase::Move,
                        delta: egui::vec2(-100.0, -80.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            } else {
                vec![]
            };
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 900.0),
                    )),
                    time: Some(frame as f64 * 0.1),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let response = super::super::geometry::window(ui.ctx())
                        .fixed_size(size)
                        .show(ui.ctx(), |ui| {
                            let current_header = ui
                                .allocate_exact_size(
                                    egui::vec2(ui.available_width(), 48.0),
                                    egui::Sense::hover(),
                                )
                                .0;
                            if frame == 3 {
                                header = current_header;
                            } else if frame > 3 {
                                assert_eq!(
                                    current_header, header,
                                    "scrolling must not move the header"
                                );
                            }
                            let output = show(ui, |ui| {
                                ui.allocate_space(ui.available_size());
                            });
                            viewport = output.inner_rect;
                            offset = output.state.offset;
                            assert!(
                                output.content_size.x >= 700.0 && output.content_size.y >= 420.0
                            );
                            assert!(
                                viewport.width() <= size.x && viewport.height() <= size.y - 48.0
                            );
                        })
                        .unwrap();
                    assert!(response.response.rect.width() <= size.x + 20.0);
                    assert!(response.response.rect.height() <= size.y + 20.0);
                },
            );
        }
        assert!(
            offset.x > 0.0 && offset.y > 0.0,
            "both axes must scroll: {offset:?}"
        );
    }
}

#[test]
fn large_windows_do_not_scroll_when_the_layout_fits() {
    let ctx = egui::Context::default();
    for _ in 0..4 {
        let _ = ctx.run_ui(Default::default(), |ui| {
            ui.set_max_size(egui::vec2(900.0, 700.0));
            let output = show(ui, |ui| {
                ui.allocate_space(ui.available_size());
            });
            assert_eq!(output.state.offset, egui::Vec2::ZERO);
            assert!(output.content_size.x <= output.inner_rect.width() + 1.0);
            assert!(output.content_size.y <= output.inner_rect.height() + 1.0);
        });
    }
}
