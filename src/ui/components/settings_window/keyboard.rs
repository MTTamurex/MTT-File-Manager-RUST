use crate::app::shortcuts::ShortcutEditorState;
use eframe::egui;

pub(super) fn settings_escape_closes_window(
    ctx: &egui::Context,
    editor: &mut ShortcutEditorState,
) -> bool {
    // Popups and creation dialogs own Escape while they are open. Settings
    // runs before those widgets, so leave the event for the topmost one.
    if !editor.is_capturing()
        && (egui::Popup::is_any_open(ctx)
            || ctx.memory(|memory| memory.top_modal_layer().is_some()))
    {
        return false;
    }
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
