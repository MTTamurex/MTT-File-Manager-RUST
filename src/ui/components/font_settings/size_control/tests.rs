use super::*;

struct Frame {
    slider: egui::Rect,
    slider_width: f32,
    output: egui::FullOutput,
}

fn frame(ctx: &egui::Context, scale: &mut f32, enabled: bool, events: Vec<egui::Event>) -> Frame {
    let mut slider = egui::Rect::NOTHING;
    let mut slider_width = 0.0;
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
            slider_width = ui.spacing().slider_width;
            let (value, response) = render(ui, "Test Font", *scale, enabled);
            slider = response.rect;
            if let Some(value) = value {
                *scale = value;
                let mut fonts = egui::FontDefinitions::default();
                sizing::apply(&mut fonts, value);
                ctx.set_fonts(fonts);
            }
        },
    );
    Frame {
        slider,
        slider_width,
        output,
    }
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
fn dragging_does_not_move_the_slider_and_applies_on_release() {
    let ctx = egui::Context::default();
    let mut scale = 1.0;
    frame(&ctx, &mut scale, true, vec![]);
    let initial = frame(&ctx, &mut scale, true, vec![]);
    let start = initial.slider.left_center() + egui::vec2(initial.slider_width * 0.35, 0.0);
    let end = initial.slider.left_center() + egui::vec2(initial.slider_width * 0.8, 0.0);
    frame(
        &ctx,
        &mut scale,
        true,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    );
    let held = frame(&ctx, &mut scale, true, vec![egui::Event::PointerMoved(end)]);
    assert_eq!(scale, 1.0);
    assert_eq!(held.slider.min, initial.slider.min);
    frame(&ctx, &mut scale, true, vec![button(end, false)]);
    assert!(scale > 1.1);
    assert_eq!(ctx.zoom_factor(), 1.0);
}

#[test]
fn reset_button_restores_one_hundred_percent() {
    let ctx = egui::Context::default();
    let mut scale = 1.25;
    frame(&ctx, &mut scale, true, vec![]);
    let initial = frame(&ctx, &mut scale, true, vec![]);
    let pos = initial
        .output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.text() == "100%" => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .unwrap();
    frame(
        &ctx,
        &mut scale,
        true,
        vec![egui::Event::PointerMoved(pos), button(pos, true)],
    );
    frame(&ctx, &mut scale, true, vec![button(pos, false)]);
    assert_eq!(scale, sizing::DEFAULT);
}
