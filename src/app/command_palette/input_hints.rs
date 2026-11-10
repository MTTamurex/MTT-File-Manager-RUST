//! Discover existing input mappings without changing or inventing shortcuts.
use super::catalog::Command;
use crate::app::shortcuts::{ShortcutAction, ShortcutBinding, ShortcutBindings, ShortcutTrigger};
use rust_i18n::t;

fn binding_label(binding: ShortcutBinding) -> String {
    let wheel = match binding.trigger {
        ShortcutTrigger::ScrollUp => t!("command_palette.wheel_up"),
        ShortcutTrigger::ScrollDown => t!("command_palette.wheel_down"),
        ShortcutTrigger::Key(eframe::egui::Key::ArrowLeft) => "←".into(),
        ShortcutTrigger::Key(eframe::egui::Key::ArrowRight) => "→".into(),
        ShortcutTrigger::Key(eframe::egui::Key::ArrowUp) => "↑".into(),
        ShortcutTrigger::Key(eframe::egui::Key::ArrowDown) => "↓".into(),
        ShortcutTrigger::Key(_) => return binding.to_string(),
    };
    format!(
        "{}{}{}{wheel}",
        if binding.ctrl { "Ctrl+" } else { "" },
        if binding.alt { "Alt+" } else { "" },
        if binding.shift { "Shift+" } else { "" },
    )
}

fn cycle_bindings(shortcuts: &ShortcutBindings) -> String {
    let next = shortcuts.get(ShortcutAction::NextViewMode);
    let previous = shortcuts.get(ShortcutAction::PreviousViewMode);
    if next.is_scroll()
        && previous.is_scroll()
        && next.trigger != previous.trigger
        && (next.ctrl, next.alt, next.shift) == (previous.ctrl, previous.alt, previous.shift)
    {
        format!(
            "{}{}{}{}",
            if next.ctrl { "Ctrl+" } else { "" },
            if next.alt { "Alt+" } else { "" },
            if next.shift { "Shift+" } else { "" },
            t!("command_palette.wheel_both"),
        )
    } else {
        format!("{} / {}", binding_label(next), binding_label(previous))
    }
}

pub(crate) fn summary(command: Command, shortcuts: &ShortcutBindings) -> String {
    let mut parts = Vec::new();
    if let Some(binding) = command.binding(shortcuts) {
        parts.push(binding_label(binding));
    }
    match command {
        Command::Back => parts.push(t!("command_palette.mouse_back").to_string()),
        Command::Forward => parts.push(t!("command_palette.mouse_forward").to_string()),
        Command::NewTab => parts.push(t!("command_palette.middle_folder").to_string()),
        Command::CloseTab => parts.push(t!("command_palette.middle_tab").to_string()),
        Command::Grid | Command::Details | Command::Columns | Command::Miller => {
            parts.push(
                t!(
                    "command_palette.cycle_binding",
                    binding = cycle_bindings(shortcuts)
                )
                .to_string(),
            );
        }
        _ => {}
    }
    parts.join("  ·  ")
}

pub(crate) fn help(command: Command, shortcuts: &ShortcutBindings) -> Option<String> {
    Some(match command {
        Command::Back => t!("command_palette.mouse_back_help").to_string(),
        Command::Forward => t!("command_palette.mouse_forward_help").to_string(),
        Command::NewTab => t!(
            "command_palette.middle_folder_help",
            binding = binding_label(shortcuts.get(ShortcutAction::NewTab)),
        )
        .to_string(),
        Command::CloseTab => t!("command_palette.middle_tab_help").to_string(),
        Command::Grid
        | Command::Details
        | Command::Columns
        | Command::Miller
        | Command::NextView
        | Command::PreviousView => t!(
            "command_palette.view_cycle_help",
            next = binding_label(shortcuts.get(ShortcutAction::NextViewMode)),
            previous = binding_label(shortcuts.get(ShortcutAction::PreviousViewMode)),
        )
        .to_string(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_hints_remain_visible_when_keyboard_is_remapped() {
        let mut shortcuts = ShortcutBindings::default();
        shortcuts.set(
            ShortcutAction::GoBack,
            ShortcutBinding::ctrl(eframe::egui::Key::B),
        );
        let hint = summary(Command::Back, &shortcuts);
        assert!(hint.contains("Ctrl+B"));
        assert!(hint.contains(t!("command_palette.mouse_back").as_ref()));
        for command in [
            Command::Back,
            Command::Forward,
            Command::NewTab,
            Command::CloseTab,
        ] {
            assert!(help(command, &shortcuts).is_some());
        }
        shortcuts.set(
            ShortcutAction::NewTab,
            ShortcutBinding::ctrl(eframe::egui::Key::N),
        );
        assert!(help(Command::NewTab, &shortcuts)
            .unwrap()
            .contains("Ctrl+N"));
    }

    #[test]
    fn view_hints_reflect_current_cycle_bindings_not_direct_view_shortcuts() {
        let mut shortcuts = ShortcutBindings::default();
        assert!(
            summary(Command::Grid, &shortcuts).contains(t!("command_palette.wheel_both").as_ref())
        );
        shortcuts.set(
            ShortcutAction::NextViewMode,
            ShortcutBinding::ctrl_shift(eframe::egui::Key::G),
        );
        shortcuts.set(
            ShortcutAction::PreviousViewMode,
            ShortcutBinding::ctrl_shift(eframe::egui::Key::H),
        );
        for command in [
            Command::Grid,
            Command::Details,
            Command::Columns,
            Command::Miller,
        ] {
            let summary = summary(command, &shortcuts);
            assert!(summary.contains("Ctrl+Shift+G"));
            assert!(summary.contains("Ctrl+Shift+H"));
            assert!(command.binding(&shortcuts).is_none());
        }
        assert_eq!(
            Command::NextView.binding(&shortcuts),
            Some(shortcuts.get(ShortcutAction::NextViewMode))
        );
        assert_eq!(
            Command::PreviousView.binding(&shortcuts),
            Some(shortcuts.get(ShortcutAction::PreviousViewMode))
        );
    }
}
