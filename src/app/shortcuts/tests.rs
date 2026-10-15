use super::*;

#[test]
fn parses_and_serializes_default_bindings() {
    for action in ShortcutAction::ALL {
        let binding = action.default_binding();
        assert_eq!(ShortcutBinding::parse(&binding.serialize()), Some(binding));
    }
}

#[test]
fn rejects_reserved_windows_shortcuts() {
    assert_eq!(
        validate_binding_policy(ShortcutBinding::alt(egui::Key::F4)),
        Err(ShortcutValidationError::Reserved)
    );
    assert_eq!(
        validate_binding_policy(ShortcutBinding::new(true, true, false, egui::Key::Delete)),
        Err(ShortcutValidationError::Reserved)
    );
}

#[test]
fn keeps_valid_swaps_from_preferences() {
    let prefs = HashMap::from([
        ("shortcut_new_tab".into(), "Ctrl+W".into()),
        ("shortcut_close_tab".into(), "Ctrl+T".into()),
    ]);
    let bindings = ShortcutBindings::from_preferences(&prefs);
    assert_eq!(
        bindings.get(ShortcutAction::NewTab),
        ShortcutBinding::ctrl(egui::Key::W)
    );
    assert_eq!(
        bindings.get(ShortcutAction::CloseTab),
        ShortcutBinding::ctrl(egui::Key::T)
    );
}

#[test]
fn duplicate_preference_falls_back_to_default_owner() {
    let prefs = HashMap::from([("shortcut_new_tab".into(), "Ctrl+W".into())]);
    let bindings = ShortcutBindings::from_preferences(&prefs);
    assert_eq!(
        bindings.get(ShortcutAction::NewTab),
        ShortcutBinding::ctrl(egui::Key::T)
    );
    assert_eq!(
        bindings.get(ShortcutAction::CloseTab),
        ShortcutBinding::ctrl(egui::Key::W)
    );
}

#[test]
fn ignores_preferences_for_fixed_file_shortcuts() {
    let prefs = HashMap::from([
        ("shortcut_copy".into(), "Ctrl+Q".into()),
        ("shortcut_delete".into(), "F8".into()),
    ]);
    let bindings = ShortcutBindings::from_preferences(&prefs);
    assert_eq!(
        bindings.get(ShortcutAction::Copy),
        ShortcutBinding::ctrl(egui::Key::C)
    );
    assert_eq!(
        bindings.get(ShortcutAction::Delete),
        ShortcutBinding::plain(egui::Key::Delete)
    );
}

#[test]
fn scroll_preferences_round_trip_and_detect_conflicts() {
    let mut bindings = ShortcutBindings::default();
    let custom = ShortcutBinding::scroll(false, true, true, false);
    bindings.set(ShortcutAction::NextViewMode, custom);
    let mut prefs = Vec::new();
    bindings.append_preferences(&mut prefs);
    let prefs = prefs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
    let restored = ShortcutBindings::from_preferences(&prefs);
    assert_eq!(restored.get(ShortcutAction::NextViewMode), custom);
    assert_eq!(
        restored.validate_candidate(ShortcutAction::PreviousViewMode, custom),
        Err(ShortcutValidationError::Conflict(
            ShortcutAction::NextViewMode
        ))
    );
    assert_eq!(
        restored.validate_candidate(
            ShortcutAction::NextViewMode,
            ShortcutBinding::scroll(true, false, true, true)
        ),
        Err(ShortcutValidationError::Reserved)
    );
    assert_eq!(
        restored.validate_candidate(
            ShortcutAction::NextViewMode,
            ShortcutBinding::scroll(false, false, false, true)
        ),
        Err(ShortcutValidationError::Unsupported)
    );
}

#[test]
fn keyboard_aliases_remain_compatible() {
    for (alias, canonical) in [
        ("control+pgup", "Ctrl+PageUp"),
        ("CTRL+pgdn", "Ctrl+PageDown"),
        ("shift+del", "Shift+Delete"),
        ("ctrl+1", "Ctrl+1"),
    ] {
        assert_eq!(
            ShortcutBinding::parse(alias),
            ShortcutBinding::parse(canonical)
        );
        assert!(ShortcutBinding::parse(alias).is_some());
    }
}

fn wheel_event(up: bool) -> egui::Event {
    egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, if up { 1.0 } else { -1.0 }),
        modifiers: egui::Modifiers::CTRL,
        phase: egui::TouchPhase::Move,
    }
}

#[test]
fn wheel_capture_and_consumption_preserve_direction_and_modifiers() {
    for (up, action) in [
        (true, ShortcutAction::PreviousViewMode),
        (false, ShortcutAction::NextViewMode),
    ] {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![wheel_event(up)],
                ..Default::default()
            },
            |ui| {
                assert_eq!(
                    capture_shortcut(ui.ctx()),
                    Some(ShortcutCapture::Binding(action.default_binding()))
                );
                assert!(ui.input(|input| input.events.is_empty()));
                assert_eq!(
                    ui.input(|input| input.smooth_scroll_delta),
                    egui::Vec2::ZERO
                );
            },
        );
        let bindings = ShortcutBindings::default();
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![wheel_event(up)],
                ..Default::default()
            },
            |ui| {
                assert!(bindings.consume_triggered(action, ui.ctx()));
                assert!(!bindings.consume_triggered(action, ui.ctx()));
                assert_eq!(
                    ui.input(|input| input.smooth_scroll_delta),
                    egui::Vec2::ZERO
                );
            },
        );
    }
}

#[test]
fn view_modes_can_use_keyboard_and_old_preferences_keep_scroll_defaults() {
    let mut bindings = ShortcutBindings::from_preferences(&HashMap::from([(
        "shortcut_new_tab".into(),
        "Ctrl+Q".into(),
    )]));
    assert_eq!(
        bindings.get(ShortcutAction::NextViewMode),
        ShortcutBinding::scroll(true, false, false, false)
    );
    let keyboard = ShortcutBinding::ctrl_shift(egui::Key::G);
    assert!(bindings
        .validate_candidate(ShortcutAction::NextViewMode, keyboard)
        .is_ok());
    bindings.set(ShortcutAction::NextViewMode, keyboard);
    let mut prefs = Vec::new();
    bindings.append_preferences(&mut prefs);
    let restored = ShortcutBindings::from_preferences(
        &prefs
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect(),
    );
    assert_eq!(restored.get(ShortcutAction::NextViewMode), keyboard);
    assert_eq!(
        restored.get(ShortcutAction::NewTab),
        ShortcutBinding::ctrl(egui::Key::Q)
    );
}

#[test]
fn restoring_one_view_mode_shortcut_cannot_duplicate_the_other() {
    let mut bindings = ShortcutBindings::default();
    let next = ShortcutAction::NextViewMode;
    let previous = ShortcutAction::PreviousViewMode;
    let custom_next = ShortcutBinding::ctrl_shift(egui::Key::G);
    bindings.set(next, custom_next);
    bindings.set(previous, next.default_binding());

    assert_eq!(
        bindings.reset(next),
        Err(ShortcutValidationError::Conflict(previous))
    );
    assert_eq!(bindings.get(next), custom_next);
    assert_eq!(bindings.get(previous), next.default_binding());
    assert!(bindings.reset(previous).is_ok());
    assert!(bindings.reset(next).is_ok());
    assert!(!bindings.any_customized());
}

#[test]
fn restoring_all_shortcuts_resolves_swapped_view_mode_bindings() {
    let mut bindings = ShortcutBindings::default();
    bindings.set(
        ShortcutAction::NextViewMode,
        ShortcutAction::PreviousViewMode.default_binding(),
    );
    bindings.set(
        ShortcutAction::PreviousViewMode,
        ShortcutAction::NextViewMode.default_binding(),
    );
    bindings.reset_all();
    for action in ShortcutAction::ALL {
        assert_eq!(bindings.get(action), action.default_binding());
    }
}
