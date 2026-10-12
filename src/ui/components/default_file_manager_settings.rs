use crate::infrastructure::windows::default_file_manager::RegistrationStatus;
use crate::ui::components::settings_ui;
use eframe::egui::{self, RichText};
use rust_i18n::t;

#[derive(Default)]
pub struct DefaultFileManagerSettingsOutput {
    pub folder_action: Option<bool>,
    pub win_e_action: Option<bool>,
}

pub fn render_default_file_manager_settings_section(
    ui: &mut egui::Ui,
    status: RegistrationStatus,
    pending: bool,
) -> DefaultFileManagerSettingsOutput {
    let mut enabled = status.is_active();
    let changed = ui
        .add_enabled_ui(!pending, |ui| {
            settings_ui::toggle_row(
                ui,
                &t!("settings.default_file_manager_description"),
                &mut enabled,
            )
        })
        .inner;

    ui.add_space(6.0);
    ui.label(
        RichText::new(t!("settings.default_file_manager_experimental_note"))
            .color(ui.visuals().warn_fg_color),
    );

    ui.add_space(12.0);
    let mut win_e_enabled = status.win_e;
    let win_e_changed = ui
        .add_enabled_ui(!pending, |ui| {
            settings_ui::toggle_row(
                ui,
                &t!("settings.win_e_open_mtt_description"),
                &mut win_e_enabled,
            )
        })
        .inner;

    ui.add_space(6.0);
    ui.label(
        RichText::new(t!("settings.win_e_open_mtt_experimental_note"))
            .color(ui.visuals().warn_fg_color),
    );

    DefaultFileManagerSettingsOutput {
        folder_action: changed.then_some(enabled),
        win_e_action: win_e_changed.then_some(win_e_enabled),
    }
}
