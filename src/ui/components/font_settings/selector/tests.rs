use super::*;

fn frame(ctx: &egui::Context, selected: &str, events: Vec<egui::Event>) -> Output {
    frame_with_families(
        ctx,
        &["Test Font A".into(), "Test Font B".into()],
        selected,
        events,
    )
}

pub(super) fn frame_with_families(
    ctx: &egui::Context,
    families: &[String],
    selected: &str,
    events: Vec<egui::Event>,
) -> Output {
    let mut result = None;
    let _ = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 700.0),
            )),
            events,
            time: Some(ctx.input(|input| input.time) + 1.0 / 60.0),
            ..Default::default()
        },
        |ui| {
            result = Some(render(ui, families, selected));
        },
    );
    result.unwrap()
}

pub(super) fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

pub(super) fn key(key: egui::Key) -> Vec<egui::Event> {
    [true, false]
        .into_iter()
        .map(|pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        })
        .collect()
}

fn open(ctx: &egui::Context, selected: &str) -> Output {
    frame(ctx, selected, vec![]);
    let pos = frame(ctx, selected, vec![]).response.rect.center();
    frame(
        ctx,
        selected,
        vec![egui::Event::PointerMoved(pos), button(pos, true)],
    );
    frame(ctx, selected, vec![button(pos, false)]);
    let output = frame(ctx, selected, vec![]);
    assert!(output.open);
    output
}

#[test]
fn arrows_preview_without_closing_and_enter_confirms() {
    let ctx = egui::Context::default();
    open(&ctx, "");
    let output = frame(&ctx, "", key(egui::Key::ArrowDown));
    assert!(output.open);
    assert_eq!(output.preview.as_deref(), Some("Test Font A"));
    assert!(output.confirmed.is_none());
    let output = frame(&ctx, "", key(egui::Key::ArrowDown));
    assert_eq!(output.preview.as_deref(), Some("Test Font B"));
    let output = frame(&ctx, "", key(egui::Key::Enter));
    assert_eq!(output.confirmed.as_deref(), Some("Test Font B"));
    assert!(!output.open);
    assert!(!output.cancelled);
}

#[test]
fn escape_cancels_without_confirming_and_reopening_starts_at_saved_font() {
    let ctx = egui::Context::default();
    open(&ctx, "Test Font A");
    frame(&ctx, "Test Font A", key(egui::Key::ArrowDown));
    let output = frame(&ctx, "Test Font A", key(egui::Key::Escape));
    assert!(output.cancelled);
    assert!(output.confirmed.is_none());
    assert!(!output.open);
    open(&ctx, "Test Font A");
    let output = frame(&ctx, "Test Font A", key(egui::Key::Enter));
    assert_eq!(output.confirmed.as_deref(), Some("Test Font A"));
}

#[test]
fn pointer_hover_previews_and_click_confirms() {
    let ctx = egui::Context::default();
    let output = open(&ctx, "");
    let point = output.rows[2].center();
    let output = frame(&ctx, "", vec![egui::Event::PointerMoved(point)]);
    assert_eq!(output.preview.as_deref(), Some("Test Font B"));
    assert!(output.open);
    assert!(output.confirmed.is_none());
    frame(&ctx, "", vec![button(point, true)]);
    let output = frame(&ctx, "", vec![button(point, false)]);
    assert_eq!(output.confirmed.as_deref(), Some("Test Font B"));
    assert!(!output.open);
}

#[test]
fn stationary_pointer_does_not_override_keyboard_navigation() {
    let ctx = egui::Context::default();
    let output = open(&ctx, "");
    frame(
        &ctx,
        "",
        vec![egui::Event::PointerMoved(output.rows[1].center())],
    );
    let output = frame(&ctx, "", key(egui::Key::ArrowDown));
    assert_eq!(output.preview.as_deref(), Some("Test Font B"));
    let output = frame(&ctx, "", vec![]);
    assert!(output.preview.is_none());
    let output = frame(&ctx, "", key(egui::Key::Enter));
    assert_eq!(output.confirmed.as_deref(), Some("Test Font B"));
}

#[test]
fn clicking_outside_cancels_without_confirming() {
    let ctx = egui::Context::default();
    open(&ctx, "");
    frame(&ctx, "", key(egui::Key::ArrowDown));
    let point = egui::pos2(800.0, 600.0);
    frame(
        &ctx,
        "",
        vec![egui::Event::PointerMoved(point), button(point, true)],
    );
    let output = frame(&ctx, "", vec![button(point, false)]);
    assert!(!output.open);
    assert!(output.confirmed.is_none());
    assert!(output.cancelled);
}

#[test]
fn up_arrow_can_preview_and_confirm_default() {
    let ctx = egui::Context::default();
    open(&ctx, "Test Font A");
    let output = frame(&ctx, "Test Font A", key(egui::Key::ArrowUp));
    assert_eq!(output.preview.as_deref(), Some(""));
    let output = frame(&ctx, "Test Font A", key(egui::Key::Enter));
    assert_eq!(output.confirmed.as_deref(), Some(""));
}
