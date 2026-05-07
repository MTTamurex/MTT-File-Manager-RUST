use crate::app::shortcuts::{
    capture_shortcut, ShortcutAction, ShortcutBindings, ShortcutCapture, ShortcutEditorState,
    ShortcutValidationError,
};
use eframe::egui;
use rust_i18n::t;

pub fn render_shortcut_settings_section(
    ui: &mut egui::Ui,
    shortcuts: &mut ShortcutBindings,
    editor_state: &mut ShortcutEditorState,
) -> bool {
    let mut changed = false;

    if let Some(action) = editor_state.capturing_action {
        if let Some(capture) = capture_shortcut(ui.ctx()) {
            match capture {
                ShortcutCapture::Cancelled => editor_state.clear(),
                ShortcutCapture::Binding(binding) => {
                    match shortcuts.validate_candidate(action, binding) {
                        Ok(()) => {
                            shortcuts.set(action, binding);
                            editor_state.clear();
                            changed = true;
                        }
                        Err(err) => {
                            editor_state.message = Some(err);
                        }
                    }
                }
            }
        }
    }

    ui.heading(t!("settings.shortcuts"));
    ui.add_space(8.0);
    ui.label(t!("settings.shortcuts_description"));
    ui.add_space(8.0);
    ui.label(egui::RichText::new(t!("settings.shortcuts_reserved_note").to_string()).small());
    ui.add_space(12.0);

    if ui
        .add_enabled(
            shortcuts.any_customized(),
            egui::Button::new(t!("settings.shortcuts_reset_all")),
        )
        .clicked()
    {
        shortcuts.reset_all();
        editor_state.clear();
        changed = true;
    }

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(8.0);

    egui::Grid::new("shortcut_settings_grid")
        .num_columns(3)
        .spacing([16.0, 8.0])
        .striped(true)
        .show(ui, |ui| {
            for action in ShortcutAction::CONFIGURABLE {
                let is_capturing = editor_state.capturing_action == Some(action);
                let button_text = if is_capturing {
                    t!("settings.shortcuts_press_binding").to_string()
                } else {
                    shortcuts.label(action)
                };

                ui.label(action_label(action));

                let button_text = if is_capturing {
                    egui::RichText::new(button_text).strong()
                } else {
                    egui::RichText::new(button_text)
                };
                if ui
                    .add_sized([220.0, 28.0], egui::Button::new(button_text))
                    .clicked()
                {
                    editor_state.begin_capture(action);
                }

                if ui
                    .add_enabled(
                        !shortcuts.is_default(action),
                        egui::Button::new(t!("settings.shortcuts_reset")),
                    )
                    .clicked()
                {
                    shortcuts.reset(action);
                    if is_capturing {
                        editor_state.clear();
                    }
                    changed = true;
                }

                ui.end_row();

                if is_capturing {
                    ui.label("");
                    ui.label(
                        egui::RichText::new(
                            t!("settings.shortcuts_press_binding_hint").to_string(),
                        )
                        .small(),
                    );
                    ui.label("");
                    ui.end_row();

                    if let Some(message) = editor_state.message {
                        ui.label("");
                        ui.colored_label(ui.visuals().error_fg_color, validation_message(message));
                        ui.label("");
                        ui.end_row();
                    }
                }
            }
        });

    ui.add_space(8.0);
    ui.separator();
    changed
}

fn action_label(action: ShortcutAction) -> String {
    match action {
        ShortcutAction::NewTab => t!("settings.shortcut_new_tab").to_string(),
        ShortcutAction::CloseTab => t!("settings.shortcut_close_tab").to_string(),
        ShortcutAction::NextTab => t!("settings.shortcut_next_tab").to_string(),
        ShortcutAction::PreviousTab => t!("settings.shortcut_previous_tab").to_string(),
        ShortcutAction::Copy => t!("settings.shortcut_copy").to_string(),
        ShortcutAction::Cut => t!("settings.shortcut_cut").to_string(),
        ShortcutAction::Paste => t!("settings.shortcut_paste").to_string(),
        ShortcutAction::Rename => t!("settings.shortcut_rename").to_string(),
        ShortcutAction::Delete => t!("settings.shortcut_delete").to_string(),
        ShortcutAction::DeletePermanently => t!("settings.shortcut_delete_permanently").to_string(),
        ShortcutAction::Refresh => t!("settings.shortcut_refresh").to_string(),
        ShortcutAction::FocusAddressBar => t!("settings.shortcut_focus_address_bar").to_string(),
        ShortcutAction::GlobalSearch => t!("settings.shortcut_global_search").to_string(),
        ShortcutAction::Properties => t!("settings.shortcut_properties").to_string(),
        ShortcutAction::CreateFolder => t!("settings.shortcut_create_folder").to_string(),
        ShortcutAction::PreviewSelected => t!("settings.shortcut_preview_selected").to_string(),
        ShortcutAction::SelectAll => t!("settings.shortcut_select_all").to_string(),
    }
}

fn validation_message(message: ShortcutValidationError) -> String {
    match message {
        ShortcutValidationError::Conflict(action) => t!(
            "settings.shortcuts_error_conflict",
            action = action_label(action)
        )
        .to_string(),
        ShortcutValidationError::Reserved => t!("settings.shortcuts_error_reserved").to_string(),
        ShortcutValidationError::Unsupported => {
            t!("settings.shortcuts_error_unsupported").to_string()
        }
    }
}
