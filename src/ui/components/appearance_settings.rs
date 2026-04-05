use crate::app::navigation_state::ThemeMode;
use eframe::egui;
use rust_i18n::t;

pub fn render_appearance_settings_section(ui: &mut egui::Ui, theme_mode: &mut ThemeMode) -> bool {
    let mut changed = false;

    ui.label(egui::RichText::new(t!("settings.appearance").to_string()).strong());
    ui.add_space(4.0);
    ui.label(t!("settings.appearance_description"));
    ui.add_space(12.0);

    let modes: &[(ThemeMode, &str)] = &[
        (ThemeMode::Light, &t!("settings.theme_light")),
        (ThemeMode::Dark, &t!("settings.theme_dark")),
    ];

    for &(mode, display_name) in modes {
        let is_selected = *theme_mode == mode;
        if ui.selectable_label(is_selected, display_name).clicked() && !is_selected {
            *theme_mode = mode;
            changed = true;
        }
    }

    ui.add_space(12.0);
    ui.label(egui::RichText::new(t!("settings.theme_apply_immediately").to_string()).small());
    ui.add_space(8.0);
    ui.separator();

    changed
}
