use crate::app::shortcuts::{
    capture_shortcut, ShortcutAction, ShortcutBindings, ShortcutCapture, ShortcutEditorState,
    ShortcutValidationError,
};
use crate::ui::theme;
use eframe::egui::{self, RichText};
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

    super::settings_ui::section_header(
        ui,
        &t!("settings.shortcuts"),
        &t!("settings.shortcuts_description"),
    );
    ui.add_space(8.0);
    ui.label(
        RichText::new(t!("settings.shortcuts_reserved_note"))
            .size(12.0)
            .color(theme::secondary_text_color(ui.ctx())),
    );
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

                ui.label(RichText::new(action_label(action)).color(theme::text_color(ui.ctx())));

                let button_text = if is_capturing {
                    RichText::new(button_text).strong()
                } else {
                    RichText::new(button_text)
                };
                if ui
                    .add_sized([220.0, 28.0], egui::Button::new(button_text))
                    .clicked()
                {
                    editor_state.begin_capture(action);
                }

                let reset_error = shortcuts
                    .validate_candidate(action, action.default_binding())
                    .err();
                let reset_response = ui.add_enabled(
                    !shortcuts.is_default(action) && reset_error.is_none(),
                    egui::Button::new(t!("settings.shortcuts_reset")),
                );
                let reset_response = if let Some(error) = reset_error {
                    reset_response.on_disabled_hover_text(validation_message(error))
                } else {
                    reset_response
                };
                if reset_response.clicked() && shortcuts.reset(action).is_ok() {
                    if is_capturing {
                        editor_state.clear();
                    }
                    changed = true;
                }

                ui.end_row();

                if is_capturing {
                    ui.label("");
                    ui.label(
                        RichText::new(t!("settings.shortcuts_press_binding_hint"))
                            .size(12.0)
                            .color(theme::secondary_text_color(ui.ctx())),
                    );
                    ui.label("");
                    ui.end_row();

                    if let Some(message) = editor_state.message {
                        ui.label("");
                        ui.add_sized(
                            [220.0, 0.0],
                            egui::Label::new(
                                RichText::new(validation_message(message))
                                    .color(ui.visuals().error_fg_color),
                            )
                            .wrap(),
                        );
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
        ShortcutAction::ReopenClosedTab => t!("settings.shortcut_reopen_closed_tab").to_string(),
        ShortcutAction::NextTab => t!("settings.shortcut_next_tab").to_string(),
        ShortcutAction::PreviousTab => t!("settings.shortcut_previous_tab").to_string(),
        ShortcutAction::MoveTabLeft => t!("settings.shortcut_move_tab_left").to_string(),
        ShortcutAction::MoveTabRight => t!("settings.shortcut_move_tab_right").to_string(),
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
        ShortcutAction::NextViewMode => t!("settings.shortcut_next_view_mode").to_string(),
        ShortcutAction::PreviousViewMode => t!("settings.shortcut_previous_view_mode").to_string(),
        ShortcutAction::GoBack => t!("settings.shortcut_go_back").to_string(),
        ShortcutAction::GoForward => t!("settings.shortcut_go_forward").to_string(),
        ShortcutAction::GoUp => t!("settings.shortcut_go_up").to_string(),
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

#[cfg(test)]
mod tests;
