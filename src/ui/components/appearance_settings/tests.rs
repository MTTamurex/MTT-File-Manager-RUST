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
        |_| {
            egui::Window::new("Appearance")
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .fixed_size(egui::vec2(700.0, 420.0))
                .show(ctx, |ui| {
                    slider_rect = render_scale_slider(ui, scale).0.rect;
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
    check_drag(1.0, 8.0);
    check_drag(2.0, 92.0);
}

fn check_drag(initial: f32, start_offset: f32) {
    let ctx = egui::Context::default();
    let mut scale = initial;
    crate::ui::interface_scale::apply(&ctx, initial);
    frame(&ctx, &mut scale, vec![]);
    let rect = frame(&ctx, &mut scale, vec![]);
    // The track is slider_width wide; the response also contains the value editor.
    let start = egui::pos2(rect.left() + start_offset, rect.center().y);
    let middle = egui::pos2(rect.left() + 50.0, rect.center().y);
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
