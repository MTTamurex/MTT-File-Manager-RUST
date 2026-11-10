use super::*;
use crate::app::command_palette::catalog::COMMANDS;

#[test]
fn every_command_has_distinct_translated_help_in_every_language() {
    let keys: std::collections::HashSet<_> =
        COMMANDS.iter().map(|command| command.help_key()).collect();
    assert_eq!(keys.len(), COMMANDS.len());
    for locale in ["en", "pt-BR", "ru", "el", "zh-CN"] {
        for command in COMMANDS {
            let key = command.help_key();
            let text = t!(key, locale = locale);
            assert_ne!(text.as_ref(), key, "{locale}: {command:?}");
            assert!(!text.trim().is_empty());
            assert_ne!(text, t!(command.label_key(), locale = locale));
        }
        let inputs = t!("command_help.inputs", locale = locale, inputs = "Ctrl+K");
        assert!(inputs.contains("Ctrl+K"));
    }
}

#[test]
fn every_command_has_help_and_keyboard_hints_follow_remappings() {
    let mut shortcuts = ShortcutBindings::default();
    for &command in COMMANDS {
        let description = t!(command.help_key());
        assert!(help(command, &shortcuts).starts_with(description.as_ref()));
        if let Some(action) = command.shortcut() {
            shortcuts.set(action, ShortcutBinding::ctrl_shift(eframe::egui::Key::K));
            let text = help(command, &shortcuts);
            assert!(text.contains("Ctrl+Shift+K"), "{command:?}: {text}");
        }
    }
    assert!(help(Command::DualPanel, &shortcuts).contains("Ctrl+Shift+D"));
    assert!(!help(Command::ClearSelection, &shortcuts).contains("%{"));
}
