use super::*;

fn wheel(y: f32, unit: egui::MouseWheelUnit, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::MouseWheel {
        delta: egui::vec2(0.0, y),
        unit,
        modifiers,
        phase: egui::TouchPhase::Move,
    }
}

fn read_frame(
    ctx: &egui::Context,
    shortcuts: &ShortcutBindings,
    mode: ViewMode,
    time: f64,
    events: Vec<egui::Event>,
) -> ViewModeInput {
    let mut result = None;
    let _ = ctx.run_ui(
        egui::RawInput {
            time: Some(time),
            events,
            ..Default::default()
        },
        |ui| {
            result = Some(read_view_mode_input(shortcuts, ui.ctx(), mode));
        },
    );
    result.unwrap()
}

#[test]
fn smooth_scroll_accumulates_and_resets_for_direction_or_idle() {
    let mut state = ScrollAccumulator::default();
    let down = ShortcutBinding::scroll(true, false, false, false);
    let up = ShortcutBinding::scroll(true, false, false, true);
    assert_eq!(state.add(down, 0.4, 1.0), 0);
    assert_eq!(state.add(down, 0.4, 1.01), 0);
    assert_eq!(state.add(down, 0.4, 1.02), 1);
    assert_eq!(state.add(up, 0.8, 1.03), 0);
    assert_eq!(state.add(up, 0.3, 2.0), 0);
    assert_eq!(state.add(up, 0.8, 2.01), 1);
}

#[test]
fn view_mode_cycle_wraps_and_reverses() {
    let modes = [
        ViewMode::Grid,
        ViewMode::List,
        ViewMode::ColumnList,
        ViewMode::Miller,
    ];
    for (index, mode) in modes.into_iter().enumerate() {
        assert_eq!(mode.cycle(true), modes[(index + 1) % 4]);
        assert_eq!(mode.cycle(false), modes[(index + 3) % 4]);
        assert_eq!(mode.cycle(true).cycle(false), mode);
    }
}

#[test]
fn batched_and_separate_wheel_steps_produce_the_same_mode() {
    let shortcuts = ShortcutBindings::default();
    let batched = egui::Context::default();
    let separate = egui::Context::default();
    let frame_events = egui::Context::default();
    let mut expected = ViewMode::Grid;
    for index in 0..3 {
        expected = read_frame(
            &separate,
            &shortcuts,
            expected,
            1.0 + index as f64 * 0.01,
            vec![wheel(
                -1.0,
                egui::MouseWheelUnit::Line,
                egui::Modifiers::CTRL,
            )],
        )
        .requested_mode;
    }
    let one_event = read_frame(
        &batched,
        &shortcuts,
        ViewMode::Grid,
        1.0,
        vec![wheel(
            -3.0,
            egui::MouseWheelUnit::Line,
            egui::Modifiers::CTRL,
        )],
    );
    let three_events = read_frame(
        &frame_events,
        &shortcuts,
        ViewMode::Grid,
        1.0,
        vec![wheel(-1.0, egui::MouseWheelUnit::Line, egui::Modifiers::CTRL); 3],
    );
    assert_eq!(expected, ViewMode::Miller);
    assert_eq!(one_event.requested_mode, expected);
    assert_eq!(three_events.requested_mode, expected);
    assert!(one_event.activity);
}

#[test]
fn pixel_scrolling_retains_fractional_steps_between_frames() {
    let ctx = egui::Context::default();
    let shortcuts = ShortcutBindings::default();
    for index in 0..3 {
        let input = read_frame(
            &ctx,
            &shortcuts,
            ViewMode::Grid,
            1.0 + index as f64 * 0.01,
            vec![wheel(
                -15.0,
                egui::MouseWheelUnit::Point,
                egui::Modifiers::CTRL,
            )],
        );
        assert_eq!(
            input.requested_mode,
            if index == 2 {
                ViewMode::List
            } else {
                ViewMode::Grid
            }
        );
        assert_eq!(input.thumbnail_delta, 0.0);
    }
}

#[test]
fn thumbnail_gesture_is_distinct_from_view_mode_switching() {
    let ctx = egui::Context::default();
    let shortcuts = ShortcutBindings::default();
    let input = read_frame(
        &ctx,
        &shortcuts,
        ViewMode::Grid,
        1.0,
        vec![wheel(
            1.0,
            egui::MouseWheelUnit::Line,
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
        )],
    );
    assert_eq!(input.requested_mode, ViewMode::Grid);
    assert!(input.thumbnail_delta > 0.0);
    assert!(ctx.input(|input| input.events.is_empty()));
    assert_eq!(
        ctx.input(|input| input.smooth_scroll_delta),
        egui::Vec2::ZERO
    );
}

#[test]
fn view_and_thumbnail_shortcuts_preserve_interface_scale() {
    let ctx = egui::Context::default();
    crate::ui::interface_scale::apply(&ctx, 1.5);
    let shortcuts = ShortcutBindings::default();
    for modifiers in [
        egui::Modifiers::CTRL,
        egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
    ] {
        read_frame(
            &ctx,
            &shortcuts,
            ViewMode::Grid,
            1.0,
            vec![wheel(1.0, egui::MouseWheelUnit::Line, modifiers)],
        );
        // A reset queued during shortcut handling takes effect on the next pass.
        read_frame(&ctx, &shortcuts, ViewMode::Grid, 1.1, vec![]);
        assert_eq!(ctx.zoom_factor(), 1.5);
    }
}

#[test]
fn ordinary_scroll_after_a_shortcut_remains_available_to_the_view() {
    let ctx = egui::Context::default();
    let shortcuts = ShortcutBindings::default();
    read_frame(
        &ctx,
        &shortcuts,
        ViewMode::Grid,
        1.0,
        vec![wheel(
            -1.0,
            egui::MouseWheelUnit::Line,
            egui::Modifiers::CTRL,
        )],
    );
    let input = read_frame(
        &ctx,
        &shortcuts,
        ViewMode::List,
        1.01,
        vec![wheel(
            -1.0,
            egui::MouseWheelUnit::Line,
            egui::Modifiers::NONE,
        )],
    );
    assert!(!input.activity);
    assert_eq!(input.requested_mode, ViewMode::List);
    assert!(ctx.input(|input| input
        .events
        .iter()
        .any(|event| matches!(event, egui::Event::MouseWheel { .. }))));
    assert_ne!(
        ctx.input(|input| input.smooth_scroll_delta),
        egui::Vec2::ZERO
    );
}

#[test]
fn custom_keyboard_shortcut_switches_mode_and_consumes_the_event() {
    let ctx = egui::Context::default();
    let mut shortcuts = ShortcutBindings::default();
    shortcuts.set(
        ShortcutAction::NextViewMode,
        ShortcutBinding::ctrl_shift(egui::Key::G),
    );
    let input = read_frame(
        &ctx,
        &shortcuts,
        ViewMode::Grid,
        1.0,
        vec![egui::Event::Key {
            key: egui::Key::G,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
        }],
    );
    assert_eq!(input.requested_mode, ViewMode::List);
    assert!(input.activity);
    assert!(ctx.input(|input| input.events.is_empty()));
}

#[test]
fn horizontal_scroll_after_a_shortcut_is_not_suppressed_by_its_smooth_tail() {
    let ctx = egui::Context::default();
    let shortcuts = ShortcutBindings::default();
    read_frame(
        &ctx,
        &shortcuts,
        ViewMode::Grid,
        1.0,
        vec![wheel(
            -1.0,
            egui::MouseWheelUnit::Line,
            egui::Modifiers::CTRL,
        )],
    );
    let input = read_frame(
        &ctx,
        &shortcuts,
        ViewMode::List,
        1.01,
        vec![egui::Event::MouseWheel {
            delta: egui::vec2(-1.0, 0.0),
            unit: egui::MouseWheelUnit::Line,
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        }],
    );
    assert!(!input.activity);
    assert_eq!(input.requested_mode, ViewMode::List);
    assert!(ctx.input(|input| input
        .events
        .iter()
        .any(|event| matches!(event, egui::Event::MouseWheel { .. }))));
    assert_ne!(ctx.input(|input| input.smooth_scroll_delta.x), 0.0);
}
