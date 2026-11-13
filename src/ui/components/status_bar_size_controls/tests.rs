use super::*;

fn frame(ctx: &egui::Context, scale: &mut f32, events: Vec<egui::Event>) -> egui::Rect {
    let mut slider_rect = egui::Rect::NOTHING;
    let _ = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1920.0, 1080.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            egui::Panel::bottom("status_bar")
                .exact_size(24.0)
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().slider_width = 150.0;
                        slider_rect = render_scale_slider(ui, scale).rect;
                    });
                });
            crate::ui::interface_scale::apply(ctx, *scale);
        },
    );
    slider_rect
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn dragging_scale_keeps_the_interface_stable_and_commits_an_intermediate_value_on_release() {
    check_drag(1.0, 0.1);
    check_drag(2.0, 0.9);
}

#[test]
fn slider_reaches_both_limits_without_a_reset_button() {
    for (fraction, expected) in [(0.0, 1.0), (1.0, 2.0)] {
        let ctx = egui::Context::default();
        let mut scale = 1.5;
        crate::ui::interface_scale::apply(&ctx, scale);
        frame(&ctx, &mut scale, vec![]);
        let rect = frame(&ctx, &mut scale, vec![]);
        let pos = egui::pos2(
            rect.left() + 1.0 + fraction * (rect.width() - 2.0),
            rect.center().y,
        );
        frame(
            &ctx,
            &mut scale,
            vec![egui::Event::PointerMoved(pos), button(pos, true)],
        );
        frame(&ctx, &mut scale, vec![button(pos, false)]);
        assert_eq!(scale, expected);
    }
}

fn check_drag(initial: f32, start_fraction: f32) {
    let ctx = egui::Context::default();
    let mut scale = initial;
    crate::ui::interface_scale::apply(&ctx, initial);
    frame(&ctx, &mut scale, vec![]);
    let rect = frame(&ctx, &mut scale, vec![]);
    let start = egui::pos2(rect.left() + rect.width() * start_fraction, rect.center().y);
    let middle = rect.center();
    frame(
        &ctx,
        &mut scale,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    );
    frame(&ctx, &mut scale, vec![egui::Event::PointerMoved(middle)]);
    assert_eq!(
        scale, initial,
        "Applying zoom while dragging displaces the slider under the pointer"
    );
    let held = frame(&ctx, &mut scale, vec![]);
    assert_eq!(ctx.zoom_factor(), initial);
    assert_eq!(held.min, rect.min);
    frame(&ctx, &mut scale, vec![button(middle, false)]);
    assert!(
        (scale - 1.5).abs() < 0.06,
        "Expected an intermediate scale, got {scale}"
    );
    frame(&ctx, &mut scale, vec![]);
    assert_eq!(ctx.zoom_factor(), scale);
}

#[test]
fn operations_stay_left_of_the_fixed_rightmost_button_in_both_layouts() {
    for (width, height, below) in [(1200.0, 24.0, false), (480.0, 72.0, true)] {
        let full = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
        let (content, button) = rects(full, 24.0);
        let (status, badge) = crate::ui::components::copy_progress_panel::indicator::rects(
            content, 180.0, 24.0, below,
        );
        assert_eq!(button.right(), full.right());
        assert_eq!(button.bottom(), full.bottom());
        assert!(badge.right() < button.left());
        assert!(!status.intersects(button));
        if below {
            assert_eq!(badge.bottom(), button.bottom());
        }
        let (_, without_operations) = rects(full, 24.0);
        assert_eq!(button, without_operations);
    }
}

fn controls_frame(ctx: &egui::Context, events: Vec<egui::Event>, time: f64) -> egui::Response {
    let mut response = None;
    let _ = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            time: Some(time),
            ..Default::default()
        },
        |ui| {
            egui::Panel::bottom("controls_test")
                .exact_size(24.0)
                .show(ui, |ui| {
                    let (_, button) = rects(ui.max_rect(), 24.0);
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(button));
                    response = Some(render(&mut child, ViewMode::Grid, &mut 128.0, &mut 1.0));
                });
        },
    );
    response.unwrap()
}

#[test]
fn hover_opens_both_controls_and_crossing_to_the_popup_keeps_it_open() {
    let ctx = egui::Context::default();
    let button = controls_frame(&ctx, vec![], 0.0);
    let button = controls_frame(
        &ctx,
        vec![egui::Event::PointerMoved(button.rect.center())],
        0.1,
    );
    let id = button.id.with("size_popup");
    assert!(egui::Popup::is_id_open(&ctx, id));
    let button = controls_frame(&ctx, vec![], 0.2);
    let popup_rect = egui::Popup::from_response(&button)
        .id(id)
        .get_popup_rect()
        .unwrap();
    assert!(popup_rect.bottom() <= button.rect.top());
    assert!(popup_rect.width() < 500.0);
    assert!(popup_rect.height() < 100.0);
    controls_frame(
        &ctx,
        vec![egui::Event::PointerMoved(popup_rect.center())],
        0.3,
    );
    controls_frame(&ctx, vec![], 0.7);
    assert!(egui::Popup::is_id_open(&ctx, id));
    controls_frame(
        &ctx,
        vec![egui::Event::PointerMoved(egui::pos2(10.0, 10.0))],
        0.8,
    );
    controls_frame(&ctx, vec![], 1.1);
    assert!(!egui::Popup::is_id_open(&ctx, id));
}

#[test]
fn escape_closes_the_popup_without_reopening_until_the_pointer_returns() {
    let ctx = egui::Context::default();
    let button = controls_frame(&ctx, vec![], 0.0);
    let pos = button.rect.center();
    let id = button.id.with("size_popup");
    controls_frame(&ctx, vec![egui::Event::PointerMoved(pos)], 0.1);
    controls_frame(
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        0.2,
    );
    controls_frame(&ctx, vec![], 0.3);
    assert!(!egui::Popup::is_id_open(&ctx, id));
    controls_frame(
        &ctx,
        vec![egui::Event::PointerMoved(egui::pos2(10.0, 10.0))],
        0.4,
    );
    controls_frame(&ctx, vec![egui::Event::PointerMoved(pos)], 0.5);
    assert!(egui::Popup::is_id_open(&ctx, id));
}

#[test]
fn popup_hides_idle_values_and_aligns_horizontal_sliders_without_modifying_them() {
    for mode in [ViewMode::Grid, ViewMode::List, ViewMode::ColumnList] {
        let ctx = egui::Context::default();
        let mut thumbnail = 128.0;
        let mut scale = 1.5;
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(320.0);
            render_popup(ui, mode, &mut thumbnail, &mut scale);
        });
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(320.0);
            let [scale_response, thumbnail_response] =
                render_popup(ui, mode, &mut thumbnail, &mut scale);
            assert!(scale_response.rect.width() > scale_response.rect.height());
            assert_eq!(scale_response.rect.left(), thumbnail_response.rect.left());
            assert_eq!(scale_response.rect.right(), thumbnail_response.rect.right());
            assert!(scale_response.rect.bottom() < thumbnail_response.rect.top());
            assert_eq!(thumbnail_response.enabled(), mode == ViewMode::Grid);
        });
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    Some(text.galley.job.text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(!texts.contains(&"128 px"));
        assert!(!texts.contains(&"150%"));
        assert_eq!(thumbnail, 128.0);
        assert_eq!(scale, 1.5);
    }
}

fn popup_frame(
    ctx: &egui::Context,
    thumbnail: &mut f32,
    scale: &mut f32,
    events: Vec<egui::Event>,
) -> ([egui::Response; 2], egui::FullOutput) {
    let mut responses = None;
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            ui.set_width(420.0);
            ui.add_space(100.0);
            responses = Some(render_popup(ui, ViewMode::Grid, thumbnail, scale));
        },
    );
    (responses.unwrap(), output)
}

fn contains_text(output: &egui::FullOutput, expected: &str) -> bool {
    output.shapes.iter().any(
        |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == expected),
    )
}

#[test]
fn values_appear_only_for_the_hovered_slider_and_hide_after_leaving() {
    let ctx = egui::Context::default();
    let mut thumbnail = 128.0;
    let mut scale = 1.5;
    let (responses, _) = popup_frame(&ctx, &mut thumbnail, &mut scale, vec![]);
    for (index, expected, hidden) in [(0, "150%", "128 px"), (1, "128 px", "150%")] {
        popup_frame(
            &ctx,
            &mut thumbnail,
            &mut scale,
            vec![egui::Event::PointerMoved(responses[index].rect.center())],
        );
        let (_, output) = popup_frame(&ctx, &mut thumbnail, &mut scale, vec![]);
        assert!(contains_text(&output, expected));
        assert!(!contains_text(&output, hidden));
    }
    let (_, output) = popup_frame(
        &ctx,
        &mut thumbnail,
        &mut scale,
        vec![egui::Event::PointerMoved(egui::pos2(10.0, 500.0))],
    );
    assert!(!contains_text(&output, "128 px"));
    assert!(!contains_text(&output, "150%"));
    assert_eq!(thumbnail, 128.0);
    assert_eq!(scale, 1.5);
}

#[test]
fn scale_tooltip_tracks_the_draft_even_when_dragging_outside_the_slider() {
    let ctx = egui::Context::default();
    let mut thumbnail = 128.0;
    let mut scale = 1.0;
    popup_frame(&ctx, &mut thumbnail, &mut scale, vec![]);
    let (responses, _) = popup_frame(&ctx, &mut thumbnail, &mut scale, vec![]);
    let rect = responses[0].rect;
    let start = rect.center();
    popup_frame(
        &ctx,
        &mut thumbnail,
        &mut scale,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    );
    let outside = egui::pos2(rect.right() + 30.0, rect.center().y);
    popup_frame(
        &ctx,
        &mut thumbnail,
        &mut scale,
        vec![egui::Event::PointerMoved(outside)],
    );
    let (_, output) = popup_frame(&ctx, &mut thumbnail, &mut scale, vec![]);
    assert!(contains_text(&output, "200%"));
    assert_eq!(scale, 1.0);
    popup_frame(
        &ctx,
        &mut thumbnail,
        &mut scale,
        vec![button(outside, false)],
    );
    assert_eq!(scale, 2.0);
    assert_eq!(thumbnail, 128.0);
}
