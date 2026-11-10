use crate::app::shortcuts::{ShortcutAction, ShortcutBinding, ShortcutBindings};
use crate::app::{navigation_state::SettingsSection, ImageViewerApp};
use crate::workers::idle_warmup::IdleWarmupMessage;
use eframe::egui;

const ACTIONS: [ShortcutAction; 4] = [
    ShortcutAction::OpenSettings,
    ShortcutAction::InvertSelection,
    ShortcutAction::TogglePreviewPanel,
    ShortcutAction::ToggleLeftSidebar,
];

pub(super) fn handle_interface_input(app: &mut ImageViewerApp, ctx: &egui::Context) -> bool {
    // Share the palette's editing, popup, modal and drag guards. Run before the
    // hardware-media handler so remapped keys cannot also control playback.
    let enabled = !app.command_palette.active
        && app.can_open_command_palette(ctx)
        && app.rectangle_selection_state.is_none();
    let (consumed, action) = read_interface_input(&app.shortcuts, ctx, enabled);
    if let Some(action) = action {
        match action {
            ShortcutAction::OpenSettings => app.open_settings(SettingsSection::General),
            ShortcutAction::InvertSelection => {
                if app.invert_current_items_selection() {
                    app.last_input = crate::app::state::LastInput::Keyboard;
                }
            }
            ShortcutAction::TogglePreviewPanel => app.toggle_preview_panel(),
            ShortcutAction::ToggleLeftSidebar => app.toggle_left_sidebar(),
            _ => unreachable!("only interface shortcuts are read"),
        }
        ctx.request_repaint();
    }
    if consumed {
        app.last_user_activity = std::time::Instant::now();
        app.refresh_working_set_trim_blocker(false);
        let _ = app
            .file_operation_state
            .idle_warmup_sender
            .send(IdleWarmupMessage::UserActive);
    }
    consumed
}

fn read_interface_input(
    shortcuts: &ShortcutBindings,
    ctx: &egui::Context,
    enabled: bool,
) -> (bool, Option<ShortcutAction>) {
    if !enabled || !ctx.input(|input| input.viewport().focused.unwrap_or(false)) {
        return (false, None);
    }
    let mut consumed = false;
    let mut action = None;
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            if let egui::Event::Key {
                key,
                modifiers,
                pressed: true,
                repeat,
                ..
            } = event
            {
                let binding = ShortcutBinding::from_modifiers(*key, *modifiers);
                if let Some(&matched) = ACTIONS
                    .iter()
                    .find(|&&candidate| shortcuts.get(candidate) == binding)
                {
                    consumed = true;
                    if !repeat && action.is_none() {
                        action = Some(matched);
                    }
                    return false;
                }
            }
            true
        });
    });
    (consumed, action)
}

#[cfg(test)]
mod tests;
