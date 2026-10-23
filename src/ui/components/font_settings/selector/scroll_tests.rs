use super::tests::{button, frame_with_families as frame, key};
use super::*;

pub(super) fn families() -> Vec<String> {
    (0..240)
        .map(|index| format!("Test Font {index:03}"))
        .collect()
}

pub(super) fn open(ctx: &egui::Context, families: &[String], selected: &str) -> Output {
    frame(ctx, families, selected, vec![]);
    let pos = frame(ctx, families, selected, vec![])
        .response
        .rect
        .center();
    frame(
        ctx,
        families,
        selected,
        vec![egui::Event::PointerMoved(pos), button(pos, true)],
    );
    frame(ctx, families, selected, vec![button(pos, false)]);
    frame(ctx, families, selected, vec![])
}

pub(super) fn assert_visible(output: &Output) {
    assert!(output.open);
    let viewport = output.viewport.unwrap();
    let row = output.rows[output.active];
    assert!(
        row.top() >= viewport.top() - 0.5 && row.bottom() <= viewport.bottom() + 0.5,
        "active={} row={row:?} viewport={viewport:?}",
        output.active
    );
}

#[test]
fn long_keyboard_navigation_keeps_the_active_row_visible_in_both_directions() {
    let ctx = egui::Context::default();
    let families = families();
    open(&ctx, &families, "");
    for direction in [egui::Key::ArrowDown, egui::Key::ArrowUp] {
        for _ in 0..180 {
            frame(&ctx, &families, "", key(direction));
            let output = frame(&ctx, &families, "", vec![]);
            assert_visible(&output);
        }
    }
}

#[test]
fn font_metric_changes_keep_a_deep_active_row_visible_without_another_keypress() {
    let ctx = egui::Context::default();
    let families = families();
    let selected = &families[160];
    open(&ctx, &families, selected);
    for _ in 0..30 {
        frame(&ctx, &families, selected, vec![]);
    }
    for scale in [1.5, 0.75, 1.25, 1.0] {
        let mut fonts = egui::FontDefinitions::default();
        crate::ui::fonts::sizing::apply(&mut fonts, scale);
        ctx.set_fonts(fonts);
        frame(&ctx, &families, selected, vec![]);
        let output = frame(&ctx, &families, selected, vec![]);
        assert_visible(&output);
    }
}

#[test]
fn manual_wheel_scrolling_is_not_pulled_back_and_arrows_resume_following() {
    let ctx = egui::Context::default();
    let families = families();
    let selected = &families[130];
    open(&ctx, &families, selected);
    frame(&ctx, &families, selected, vec![]);
    let initial = frame(&ctx, &families, selected, vec![]);
    assert_visible(&initial);
    let point = initial.rows[initial.active].center();
    frame(
        &ctx,
        &families,
        selected,
        vec![egui::Event::PointerMoved(point)],
    );
    frame(
        &ctx,
        &families,
        selected,
        vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            delta: egui::vec2(0.0, -200.0),
            modifiers: egui::Modifiers::NONE,
        }],
    );
    for _ in 0..20 {
        frame(&ctx, &families, selected, vec![]);
    }
    let scrolled = frame(&ctx, &families, selected, vec![]);
    assert!(scrolled.rows[0].top() < initial.rows[0].top() - 100.0);
    frame(&ctx, &families, selected, key(egui::Key::ArrowDown));
    let output = frame(&ctx, &families, selected, vec![]);
    assert_visible(&output);
}
