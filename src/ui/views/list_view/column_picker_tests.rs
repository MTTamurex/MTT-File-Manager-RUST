use super::*;

#[test]
fn picker_uses_compact_width_and_keeps_the_scrollbar_at_the_content_right_edge() {
    let context = egui::Context::default();
    for _ in 0..3 {
        let _ = context.run_ui(Default::default(), |ui| {
            let window =
                picker_window(ui.ctx(), egui::Id::new("test_picker")).show(ui.ctx(), |ui| {
                    let width = ui.available_width();
                    assert!(
                        width <= 441.0,
                        "picker must not expand to unrelated viewport width"
                    );
                    let right = ui.available_rect_before_wrap().right();
                    let output = choices_scroll(ui, |ui| {
                        for _ in 0..100 {
                            ui.checkbox(&mut false, "Example property");
                        }
                    });
                    assert!(
                        right - output.inner_rect.right() < 24.0,
                        "scrollbar must not sit in the middle of the picker"
                    );
                    assert!(output.content_size.y > output.inner_rect.height());
                });
            assert!(window.is_some());
        });
    }
}

#[test]
fn long_property_labels_and_search_text_do_not_expand_the_picker_or_displace_its_scrollbar() {
    let context = egui::Context::default();
    let mut search = "Example search text ".repeat(30);
    for _ in 0..3 {
        let _ = context.run_ui(Default::default(), |ui| {
            picker_window(ui.ctx(), egui::Id::new("long_labels_picker")).show(ui.ctx(), |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                let available = ui.available_width();
                let search_response =
                    ui.add(egui::TextEdit::singleline(&mut search).desired_width(f32::INFINITY));
                assert!((search_response.rect.width() - available).abs() < 2.0);
                let right = ui.available_rect_before_wrap().right();
                let output = choices_scroll(ui, |ui| {
                    for _ in 0..30 {
                        ui.checkbox(
                            &mut false,
                            "Example long property label with several descriptive words ".repeat(4),
                        );
                    }
                });
                assert!(ui.min_rect().width() <= 441.0);
                assert!(right - output.inner_rect.right() < 24.0);
            });
        });
    }
}

#[test]
fn picker_opens_centered_and_recenters_after_being_closed() {
    let ctx = egui::Context::default();
    let id = egui::Id::new("centered_picker");
    for _ in 0..3 {
        let _ = ctx.run_ui(Default::default(), |ui| {
            let window = picker_window(ui.ctx(), id)
                .show(ui.ctx(), |ui| {
                    ui.label("Example property");
                })
                .unwrap();
            assert!(
                window
                    .response
                    .rect
                    .center()
                    .distance(ui.ctx().viewport_rect().center())
                    < 2.0
            );
            assert!(window.inner.is_some(), "picker is not collapsed");
        });
    }
    let displaced = ctx.viewport_rect().center() + egui::vec2(80.0, 60.0);
    let _ = ctx.run_ui(Default::default(), |ui| {
        picker_window(ui.ctx(), id)
            .movable(false)
            .current_pos(displaced)
            .show(ui.ctx(), |ui| {
                ui.label("Example property");
            });
    });
    let _ = ctx.run_ui(Default::default(), |ui| {
        let window = picker_window(ui.ctx(), id)
            .show(ui.ctx(), |ui| {
                ui.label("Example property");
            })
            .unwrap();
        assert!(
            window.response.rect.center().distance(displaced) < 2.0,
            "open picker remains movable"
        );
    });
    for _ in 0..2 {
        let _ = ctx.run_ui(Default::default(), |_| {});
    }
    ctx.data_mut(|data| data.insert_temp(id.with("center_on_open"), true));
    let _ = ctx.run_ui(Default::default(), |ui| {
        let window = picker_window(ui.ctx(), id)
            .show(ui.ctx(), |ui| {
                ui.label("Example property");
            })
            .unwrap();
        assert!(
            window
                .response
                .rect
                .center()
                .distance(ui.ctx().viewport_rect().center())
                < 2.0
        );
    });
}
