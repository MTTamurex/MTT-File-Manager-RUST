use super::*;

const ACTIONS: [ShortcutAction; 4] = [
    ShortcutAction::OpenSettings,
    ShortcutAction::InvertSelection,
    ShortcutAction::TogglePreviewPanel,
    ShortcutAction::ToggleLeftSidebar,
];

#[test]
fn interface_defaults_are_configurable_valid_unique_and_round_trip() {
    let bindings = ShortcutBindings::from_preferences(&HashMap::new());
    assert_eq!(bindings.label(ShortcutAction::OpenSettings), "Ctrl+,");
    assert_eq!(
        ShortcutBinding::parse("Ctrl+Comma"),
        Some(ShortcutBinding::ctrl(egui::Key::Comma))
    );
    for (action, expected) in ACTIONS.into_iter().zip([
        ShortcutBinding::ctrl(egui::Key::Comma),
        ShortcutBinding::ctrl(egui::Key::I),
        ShortcutBinding::alt(egui::Key::P),
        ShortcutBinding::ctrl(egui::Key::B),
    ]) {
        assert!(ShortcutAction::CONFIGURABLE.contains(&action));
        assert_eq!(bindings.get(action), expected);
        assert_eq!(
            ShortcutBinding::parse(&expected.serialize()),
            Some(expected)
        );
        assert!(bindings.validate_candidate(action, expected).is_ok());
        for locale in ["en", "pt-BR", "ru", "el", "zh-CN"] {
            let key = action.translation_key();
            assert_ne!(rust_i18n::t!(key, locale = locale).as_ref(), key);
        }
    }
    for action in ShortcutAction::ALL {
        assert_eq!(bindings.find_conflict(action, bindings.get(action)), None);
    }
}

#[test]
fn interface_remapping_persists_and_detects_conflicts() {
    for action in ACTIONS {
        let mut bindings = ShortcutBindings::default();
        let custom = ShortcutBinding::ctrl_shift(egui::Key::K);
        assert!(bindings.validate_candidate(action, custom).is_ok());
        bindings.set(action, custom);
        assert_eq!(
            bindings.validate_candidate(ShortcutAction::NewTab, custom),
            Err(ShortcutValidationError::Conflict(action))
        );
        let restored = round_trip(&bindings);
        assert_eq!(restored.get(action), custom);
        assert_eq!(
            restored.get(ShortcutAction::PreviewSelected),
            ShortcutBinding::plain(egui::Key::Space)
        );
    }
}

#[test]
fn new_defaults_never_replace_legacy_assignments() {
    for new_action in ACTIONS {
        let legacy_action = ShortcutAction::GlobalSearch;
        let preferences = [(
            legacy_action.preference_key().to_owned(),
            new_action.default_binding().serialize(),
        )]
        .into_iter()
        .collect();
        let bindings = ShortcutBindings::from_preferences(&preferences);
        assert_eq!(bindings.get(legacy_action), new_action.default_binding());
        assert_ne!(bindings.get(new_action), new_action.default_binding());
        for action in ShortcutAction::ALL {
            assert_eq!(bindings.find_conflict(action, bindings.get(action)), None);
            assert_eq!(round_trip(&bindings).get(action), bindings.get(action));
        }
    }
}

#[test]
fn simultaneous_legacy_conflicts_get_distinct_free_bindings() {
    let legacy = [
        ShortcutAction::GlobalSearch,
        ShortcutAction::NewTab,
        ShortcutAction::GoBack,
        ShortcutAction::Refresh,
    ];
    let preferences = legacy
        .into_iter()
        .zip(ACTIONS)
        .map(|(old, new)| {
            (
                old.preference_key().to_owned(),
                new.default_binding().serialize(),
            )
        })
        .collect();
    let bindings = ShortcutBindings::from_preferences(&preferences);
    for (old, new) in legacy.into_iter().zip(ACTIONS) {
        assert_eq!(bindings.get(old), new.default_binding());
    }
    for action in ShortcutAction::ALL {
        assert_eq!(bindings.find_conflict(action, bindings.get(action)), None);
        assert_eq!(round_trip(&bindings).get(action), bindings.get(action));
    }
}

fn round_trip(bindings: &ShortcutBindings) -> ShortcutBindings {
    let mut saved = Vec::new();
    bindings.append_preferences(&mut saved);
    ShortcutBindings::from_preferences(
        &saved
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}
