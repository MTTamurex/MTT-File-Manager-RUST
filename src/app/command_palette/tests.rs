use super::{
    availability::{disabled_reason, CommandContext},
    catalog::{Command, COMMANDS},
    CommandPaletteState,
};
use std::collections::HashSet;

#[test]
fn catalog_has_unique_ids_and_translation_keys() {
    assert_eq!(
        COMMANDS.iter().collect::<HashSet<_>>().len(),
        COMMANDS.len()
    );
    assert_eq!(
        COMMANDS
            .iter()
            .map(|command| command.label_key())
            .collect::<HashSet<_>>()
            .len(),
        COMMANDS.len()
    );
}

#[test]
fn commands_show_current_bindings_including_the_fixed_panel_shortcut() {
    use crate::app::shortcuts::{ShortcutAction, ShortcutBinding, ShortcutBindings};
    let mut bindings = ShortcutBindings::default();
    let custom = ShortcutBinding::ctrl_shift(eframe::egui::Key::K);
    bindings.set(ShortcutAction::GlobalSearch, custom);
    assert_eq!(Command::GlobalSearch.binding(&bindings), Some(custom));
    assert_eq!(
        Command::DualPanel.binding(&bindings),
        Some(ShortcutBinding::ctrl_shift(eframe::egui::Key::D))
    );
    assert_eq!(Command::Settings.binding(&bindings), None);
}

#[test]
fn catalog_is_translated_in_every_supported_language() {
    for locale in ["en", "pt-BR", "ru", "el", "zh-CN"] {
        for command in COMMANDS {
            for key in [command.label_key(), command.category_key()] {
                assert_ne!(
                    rust_i18n::t!(key, locale = locale).as_ref(),
                    key,
                    "{locale}: {key}"
                );
            }
        }
    }
}

#[test]
fn selection_navigation_and_locked_views_respect_context() {
    let mut context = CommandContext::default();
    for command in [
        Command::Rename,
        Command::Properties,
        Command::ClearSelection,
        Command::SelectAll,
        Command::Back,
        Command::Up,
    ] {
        assert!(disabled_reason(command, &context).is_some(), "{command:?}");
    }
    context.has_selection = true;
    context.has_items = true;
    context.can_rename = true;
    context.can_properties = true;
    context.can_back = true;
    context.can_up = true;
    for command in [
        Command::Rename,
        Command::Properties,
        Command::ClearSelection,
        Command::SelectAll,
        Command::Back,
        Command::Up,
    ] {
        assert!(disabled_reason(command, &context).is_none(), "{command:?}");
    }
    context.view_locked = true;
    for command in [
        Command::Grid,
        Command::Details,
        Command::Columns,
        Command::Miller,
        Command::NextView,
        Command::PreviousView,
    ] {
        assert_eq!(
            disabled_reason(command, &context),
            Some("command_palette.view_locked")
        );
    }
}

#[test]
fn tabs_and_virtual_locations_are_guarded() {
    let mut context = CommandContext {
        can_add_tab: true,
        tab_pinned: true,
        ..Default::default()
    };
    assert!(disabled_reason(Command::CloseTab, &context).is_some());
    assert!(disabled_reason(Command::ReopenTab, &context).is_some());
    assert!(disabled_reason(Command::CreateFolder, &context).is_some());
    context.has_closed_tabs = true;
    assert!(disabled_reason(Command::ReopenTab, &context).is_none());
    context.can_add_tab = false;
    assert!(disabled_reason(Command::ReopenTab, &context).is_some());
    assert!(disabled_reason(Command::NewTab, &context).is_some());
}

#[test]
fn transient_state_resets_and_keyboard_selection_is_bounded() {
    let mut state = CommandPaletteState::default();
    state.open(None);
    state.query = "settings".into();
    state.move_selection(3, true);
    assert_eq!(state.selected, 2);
    state.move_selection(3, false);
    assert_eq!(state.selected, 0);
    state.move_selection(0, true);
    assert_eq!(state.selected, 0);
    state.close(&eframe::egui::Context::default());
    assert!(!state.active);
    assert!(state.query.is_empty());
    state.open(None);
    assert!(state.request_focus);
    assert!(state.query.is_empty());
}
