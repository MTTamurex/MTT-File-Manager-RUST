use super::*;

#[test]
fn image_only_senses_drag_when_it_can_pan() {
    assert!(!image_interaction_sense(false).senses_drag());
    assert!(image_interaction_sense(false).senses_click());
    assert!(image_interaction_sense(true).senses_drag());
    assert!(image_interaction_sense(true).senses_click());
}

#[test]
fn zoom_consumes_wheel_delta() {
    let ctx = egui::Context::default();
    let consumed = std::cell::Cell::new(0.0);
    let remaining = std::cell::Cell::new(egui::Vec2::ZERO);
    let mut input = egui::RawInput::default();
    input.events.push(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(12.0, 30.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });

    let _ = ctx.run_ui(input, |ui| {
        consumed.set(take_zoom_wheel_delta(ui));
        remaining.set(ui.input(|i| i.smooth_scroll_delta()));
    });

    assert!(consumed.get().abs() > f32::EPSILON);
    assert_eq!(remaining.get(), egui::Vec2::ZERO);
}
