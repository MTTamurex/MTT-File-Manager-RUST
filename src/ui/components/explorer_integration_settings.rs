use crate::ui::components::settings_ui;
use eframe::egui;
use rust_i18n::t;

#[derive(Default)]
pub struct ExplorerIntegrationSettingsOutput {
    pub default_file_manager_changed: bool,
    pub context_menu_changed: bool,
}

pub fn render_explorer_integration_settings_section(
    ui: &mut egui::Ui,
    use_mtt_as_default_file_manager: &mut bool,
    add_open_in_mtt_context_menu: &mut bool,
) -> ExplorerIntegrationSettingsOutput {
    settings_ui::section_header(
        ui,
        &t!("settings.explorer_integration_title"),
        &t!("settings.explorer_integration_description"),
    );

    let default_file_manager_changed = settings_ui::toggle_row(
        ui,
        &t!("settings.default_file_manager"),
        use_mtt_as_default_file_manager,
    );
    ui.add_space(6.0);
    ui.label(t!("settings.default_file_manager_description"));
    ui.add_space(14.0);

    let context_menu_changed = settings_ui::toggle_row(
        ui,
        &t!("settings.open_in_mtt_context_menu"),
        add_open_in_mtt_context_menu,
    );
    ui.add_space(6.0);
    ui.label(t!("settings.open_in_mtt_context_menu_description"));

    ExplorerIntegrationSettingsOutput {
        default_file_manager_changed,
        context_menu_changed,
    }
}
