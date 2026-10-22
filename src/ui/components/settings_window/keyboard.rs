use crate::app::shortcuts::ShortcutEditorState;
use eframe::egui;

pub(super) fn settings_escape_closes_window(
    ctx: &egui::Context,
    editor: &mut ShortcutEditorState,
) -> bool {
    // Combo boxes used inside settings own Escape while their popup is open.
    // The window runs before those widgets, so leave the event for the popup.
    if !editor.is_capturing() && egui::Popup::is_any_open(ctx) {
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
