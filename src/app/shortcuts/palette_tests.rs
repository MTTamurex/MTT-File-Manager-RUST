use super::*;

#[test]
fn palette_binding_round_trips_and_detects_conflicts() {
    let mut bindings = ShortcutBindings::default();
    let action = ShortcutAction::CommandPalette;
    assert_eq!(
        bindings.get(action),
        ShortcutBinding::ctrl_shift(egui::Key::P)
    );
    assert_eq!(
        bindings.validate_candidate(ShortcutAction::NewTab, bindings.get(action)),
        Err(ShortcutValidationError::Conflict(action))
    );
    let custom = ShortcutBinding::ctrl_shift(egui::Key::K);
    bindings.set(action, custom);
    let mut preferences = Vec::new();
    bindings.append_preferences(&mut preferences);
    let preferences = preferences
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
    assert_eq!(
        ShortcutBindings::from_preferences(&preferences).get(action),
        custom
    );
}

#[test]
fn legacy_customization_keeps_new_palettes_default_binding() {
    let custom = ShortcutBinding::ctrl_shift(egui::Key::P);
    let preferences = [(
        ShortcutAction::GlobalSearch.preference_key().to_owned(),
        custom.serialize(),
    )]
    .into_iter()
    .collect();
    let bindings = ShortcutBindings::from_preferences(&preferences);
    assert_eq!(bindings.get(ShortcutAction::GlobalSearch), custom);
    assert_ne!(bindings.get(ShortcutAction::CommandPalette), custom);
    let mut saved = Vec::new();
    bindings.append_preferences(&mut saved);
    let saved = saved
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
    let restored = ShortcutBindings::from_preferences(&saved);
    assert_eq!(restored.get(ShortcutAction::GlobalSearch), custom);
    assert_eq!(
        restored.get(ShortcutAction::CommandPalette),
        bindings.get(ShortcutAction::CommandPalette)
    );
}
