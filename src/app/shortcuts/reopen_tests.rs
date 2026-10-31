use super::*;

#[test]
fn reopen_shortcut_consumes_ctrl_shift_t_without_opening_a_new_tab() {
    let bindings = ShortcutBindings::default();
    assert_eq!(
        bindings.get(ShortcutAction::ReopenClosedTab),
        ShortcutBinding::ctrl_shift(egui::Key::T)
    );
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::T,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
            }],
            ..Default::default()
        },
        |ui| {
            assert!(!bindings.is_triggered(ShortcutAction::NewTab, ui.ctx()));
            assert!(bindings.consume_triggered(ShortcutAction::ReopenClosedTab, ui.ctx()));
            assert!(!bindings.is_triggered(ShortcutAction::ReopenClosedTab, ui.ctx()));
        },
    );
}

#[test]
fn reopen_shortcut_can_be_customized_and_persisted() {
    let mut bindings = ShortcutBindings::default();
    let action = ShortcutAction::ReopenClosedTab;
    let custom = ShortcutBinding::ctrl_shift(egui::Key::R);
    assert!(bindings.validate_candidate(action, custom).is_ok());
    bindings.set(action, custom);
    let mut prefs = Vec::new();
    bindings.append_preferences(&mut prefs);
    let prefs = prefs
        .into_iter()
        .map(|(key, value)| (key.into(), value))
        .collect();
    let mut restored = ShortcutBindings::from_preferences(&prefs);
    assert_eq!(restored.get(action), custom);
    assert!(restored.reset(action).is_ok());
    assert_eq!(restored.get(action), action.default_binding());
}

#[test]
fn reopen_shortcut_participates_in_conflict_detection() {
    let bindings = ShortcutBindings::default();
    assert_eq!(
        bindings.validate_candidate(
            ShortcutAction::NewTab,
            ShortcutAction::ReopenClosedTab.default_binding()
        ),
        Err(ShortcutValidationError::Conflict(
            ShortcutAction::ReopenClosedTab
        ))
    );
}
