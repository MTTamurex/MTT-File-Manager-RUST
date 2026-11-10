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
    for (command, action) in [
        (Command::Settings, ShortcutAction::OpenSettings),
        (Command::InvertSelection, ShortcutAction::InvertSelection),
        (Command::Preview, ShortcutAction::TogglePreviewPanel),
        (Command::Sidebar, ShortcutAction::ToggleLeftSidebar),
        (Command::PreviewSelected, ShortcutAction::PreviewSelected),
        (Command::Copy, ShortcutAction::Copy),
        (Command::Cut, ShortcutAction::Cut),
        (Command::Paste, ShortcutAction::Paste),
        (Command::Delete, ShortcutAction::Delete),
    ] {
        assert_eq!(command.binding(&bindings), Some(action.default_binding()));
        bindings.set(action, custom);
        assert_eq!(command.binding(&bindings), Some(custom));
    }
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

#[test]
fn file_commands_require_selection_and_operation_capabilities() {
    let mut context = CommandContext::default();
    for command in [
        Command::PreviewSelected,
        Command::Copy,
        Command::Cut,
        Command::Delete,
    ] {
        assert_eq!(
            disabled_reason(command, &context),
            Some("command_palette.no_selection")
        );
    }
    context.has_selection = true;
    assert_eq!(
        disabled_reason(Command::PreviewSelected, &context),
        Some("command_palette.preview_unavailable")
    );
    for command in [Command::Copy, Command::Cut, Command::Delete] {
        assert_eq!(
            disabled_reason(command, &context),
            Some("command_palette.unavailable")
        );
    }
    context.can_preview_selected = true;
    context.can_copy = true;
    context.can_cut = true;
    context.can_delete = true;
    for command in [
        Command::PreviewSelected,
        Command::Copy,
        Command::Cut,
        Command::Delete,
    ] {
        assert_eq!(disabled_reason(command, &context), None);
    }
    context.can_cut = false;
    context.can_delete = false;
    assert_eq!(disabled_reason(Command::Copy, &context), None);
    assert!(disabled_reason(Command::Cut, &context).is_some());
    assert!(disabled_reason(Command::Delete, &context).is_some());
}

#[test]
fn paste_requires_clipboard_files_and_a_supported_current_destination_not_selection() {
    let mut context = CommandContext::default();
    assert_eq!(
        disabled_reason(Command::Paste, &context),
        Some("command_palette.clipboard_empty")
    );
    context.has_clipboard_files = true;
    assert_eq!(
        disabled_reason(Command::Paste, &context),
        Some("command_palette.unavailable")
    );
    context.can_paste = true;
    assert_eq!(disabled_reason(Command::Paste, &context), None);
    context.has_selection = true;
    assert_eq!(disabled_reason(Command::Paste, &context), None);
    for locale in ["en", "pt-BR", "ru", "el", "zh-CN"] {
        for key in [
            "command_palette.preview_unavailable",
            "command_palette.clipboard_empty",
        ] {
            assert_ne!(rust_i18n::t!(key, locale = locale).as_ref(), key);
        }
    }
}

#[test]
fn disk_analysis_requires_an_eligible_current_drive_but_not_a_selection_or_shortcut() {
    use crate::app::shortcuts::ShortcutBindings;
    let mut context = CommandContext::default();
    assert_eq!(
        disabled_reason(Command::DiskAnalysis, &context),
        Some("command_palette.disk_analysis_unavailable")
    );
    // A selected item alone is not a current drive, notably in virtual views.
    context.has_selection = true;
    assert!(disabled_reason(Command::DiskAnalysis, &context).is_some());
    context.has_selection = false;
    context.disk_analysis_drive = Some('D');
    assert_eq!(disabled_reason(Command::DiskAnalysis, &context), None);
    assert_eq!(Command::DiskAnalysis.shortcut(), None);
    assert_eq!(
        Command::DiskAnalysis.binding(&ShortcutBindings::default()),
        None
    );
    assert!(
        super::input_hints::summary(Command::DiskAnalysis, &ShortcutBindings::default()).is_empty()
    );
    for locale in ["en", "pt-BR", "ru", "el", "zh-CN"] {
        assert_ne!(
            rust_i18n::t!("command_palette.disk_analysis_unavailable", locale = locale).as_ref(),
            "command_palette.disk_analysis_unavailable"
        );
    }
}
