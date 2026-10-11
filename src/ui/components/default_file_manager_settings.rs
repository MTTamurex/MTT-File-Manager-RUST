use crate::infrastructure::windows::default_file_manager::RegistrationStatus;
use crate::ui::components::settings_ui;
use eframe::egui::{self, RichText};
use rust_i18n::t;

pub fn render_default_file_manager_settings_section(
    ui: &mut egui::Ui,
    status: RegistrationStatus,
    pending: bool,
) -> Option<bool> {
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

    changed.then_some(enabled)
}
