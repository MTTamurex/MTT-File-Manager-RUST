use crate::app::navigation_state::QuickAccessPlacement;
use crate::app::ImageViewerApp;
use crate::ui::components::settings_ui;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

pub struct GeneralSettingsOutput {
    pub language_changed: bool,
    pub theme_changed: bool,
    pub startup_date_changed: bool,
    pub quick_access_changed: bool,
    pub recycle_bin_changed: bool,
    pub minimize_to_tray_changed: bool,
    pub default_file_manager_action: Option<bool>,
    pub win_e_action: Option<bool>,
}

pub fn render_general_settings_section(
    ui: &mut egui::Ui,
    app: &mut ImageViewerApp,
) -> GeneralSettingsOutput {
    let dark_mode = ui.visuals().dark_mode;
    let mut output = GeneralSettingsOutput {
        language_changed: false,
        theme_changed: false,
        startup_date_changed: false,
        quick_access_changed: false,
        recycle_bin_changed: false,
        minimize_to_tray_changed: false,
        default_file_manager_action: None,
        win_e_action: None,
    };

    output.language_changed |=
        crate::ui::components::language_settings::render_language_settings_section(ui);
    ui.add_space(20.0);
    output.theme_changed |=
        crate::ui::components::appearance_settings::render_appearance_settings_section(
            ui,
            &mut app.theme_mode,
            &mut app.interface_scale,
        );
    ui.add_space(20.0);
    output.startup_date_changed = crate::ui::components::startup_date_settings::render(ui, app);
    ui.add_space(20.0);
    if settings_ui::toggle_row(
        ui,
        &t!("settings.show_quick_access_sidebar"),
        &mut app.show_quick_access,
    ) {
        output.quick_access_changed = true;
    }
    ui.add_space(6.0);
    ui.label(
        RichText::new(t!("settings.quick_access_position"))
            .size(13.0)
            .color(theme::secondary_text_color(dark_mode)),
    );
    let placements: [(QuickAccessPlacement, &str); 2] = [
        (
            QuickAccessPlacement::Sidebar,
            &t!("settings.quick_access_position_sidebar"),
        ),
        (
            QuickAccessPlacement::Horizontal,
            &t!("settings.quick_access_position_horizontal"),
        ),
    ];
    let selected = placements
        .iter()
        .position(|(placement, _)| *placement == app.quick_access_placement)
        .unwrap_or(0);
    let labels: Vec<&str> = placements.iter().map(|(_, label)| *label).collect();
    if let Some(index) = settings_ui::segmented_choice(ui, &labels, selected) {
        app.quick_access_placement = placements[index].0;
        output.quick_access_changed = true;
    }
    ui.add_space(10.0);
    if settings_ui::toggle_row(
        ui,
        &t!("settings.show_recycle_bin"),
        &mut app.show_recycle_bin,
    ) {
        output.recycle_bin_changed = true;
    }
    ui.add_space(12.0);
    if settings_ui::toggle_row(
        ui,
        &t!("settings.minimize_to_tray"),
        &mut app.minimize_to_tray,
    ) {
        output.minimize_to_tray_changed = true;
    }
    ui.add_space(20.0);
    let default_file_manager_settings =
        crate::ui::components::default_file_manager_settings::render_default_file_manager_settings_section(
            ui,
            app.default_file_manager_settings.status,
            app.default_file_manager_settings.result_receiver.is_some(),
        );
    output.default_file_manager_action = default_file_manager_settings.folder_action;
    output.win_e_action = default_file_manager_settings.win_e_action;

    output
}
