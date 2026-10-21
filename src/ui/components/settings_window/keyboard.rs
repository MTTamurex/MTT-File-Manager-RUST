use crate::app::shortcuts::ShortcutEditorState;
use eframe::egui;

pub(super) fn settings_escape_closes_window(
    ctx: &egui::Context,
    editor: &mut ShortcutEditorState,
) -> bool {
    let pressed = ctx.input_mut(|input| {
        let mut pressed = false;
        input.events.retain(|event| {
            if let egui::Event::Key {
                key: egui::Key::Escape,
                pressed: true,
                repeat,
                ..
            } = event
            {
                pressed |= !repeat;
                return false;
            }
            true
        });
        pressed
    });
    if pressed && editor.is_capturing() {
        editor.clear();
        return false;
    }
    pressed
}
