use crate::app::shortcuts::{ShortcutAction, ShortcutBinding, ShortcutBindings};
use crate::app::ImageViewerApp;
use eframe::egui;

pub(super) fn handle_navigation_input(
    app: &mut ImageViewerApp,
    ctx: &egui::Context,
    text_input_active: bool,
) -> bool {
    // Preserve context-menu dismissal and do not retarget an in-progress drop.
    if app.context_menu.is_open || app.is_item_dragging {
        return false;
    }

    let input = read_navigation_input(&app.shortcuts, ctx, text_input_active);
    if input.back && app.can_go_back() {
        app.cancel_rectangle_selection();
        app.go_back();
    }
    if input.forward && app.can_go_forward() {
        app.cancel_rectangle_selection();
        app.go_forward();
    }
    if input.up && app.can_go_up() {
        app.cancel_rectangle_selection();
        app.go_up_one_level();
    }
    input.back || input.forward || input.up
}

#[derive(Default)]
struct NavigationInput {
    back: bool,
    forward: bool,
    up: bool,
}

fn read_navigation_input(
    shortcuts: &ShortcutBindings,
    ctx: &egui::Context,
    text_input_active: bool,
) -> NavigationInput {
    let mut navigation = NavigationInput::default();
    ctx.input_mut(|input| {
        let keyboard_enabled = !text_input_active && input.viewport().focused.unwrap_or(false);
        input.events.retain(|event| {
            // Mouse buttons remain fixed and keep their existing behavior.
            match event {
                egui::Event::PointerButton {
                    button: egui::PointerButton::Extra1,
                    pressed: true,
                    ..
                } => navigation.back = true,
                egui::Event::PointerButton {
                    button: egui::PointerButton::Extra2,
                    pressed: true,
                    ..
                } => navigation.forward = true,
                egui::Event::Key {
                    key,
                    modifiers,
                    pressed: true,
                    repeat,
                    ..
                } if keyboard_enabled => {
                    let binding = ShortcutBinding::from_modifiers(*key, *modifiers);
                    for (action, triggered) in [
                        (ShortcutAction::GoBack, &mut navigation.back),
                        (ShortcutAction::GoForward, &mut navigation.forward),
                        (ShortcutAction::GoUp, &mut navigation.up),
                    ] {
                        if binding == shortcuts.get(action) {
                            // Consume repeats as well, but navigate only once per press.
                            *triggered |= !repeat;
                            return false;
                        }
                    }
                }
                _ => {}
            }
            true
        });
    });
    navigation
}

#[cfg(test)]
mod tests;
