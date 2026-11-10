use eframe::egui;

pub(crate) fn consume_toggle(
    shortcuts: &crate::app::shortcuts::ShortcutBindings,
    ctx: &egui::Context,
) -> bool {
    use crate::app::shortcuts::{ShortcutAction, ShortcutBinding};
    let binding = shortcuts.get(ShortcutAction::CommandPalette);
    let fresh_press = ctx.input(|input| {
        input.events.iter().any(|event| {
            matches!(event, egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. }
                if ShortcutBinding::from_modifiers(*key, *modifiers) == binding)
        })
    });
    shortcuts.consume_triggered(ShortcutAction::CommandPalette, ctx) && fresh_press
}

pub(super) struct Keys {
    pub up: bool,
    pub down: bool,
    pub run: bool,
    pub close: bool,
}

pub(super) fn read(ctx: &egui::Context) -> Keys {
    ctx.input_mut(|input| Keys {
        up: input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
        down: input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
        run: input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
        close: input.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
    })
}
