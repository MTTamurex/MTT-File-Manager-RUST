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
                        ui.spacing_mut().slider_width = 56.0;
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
