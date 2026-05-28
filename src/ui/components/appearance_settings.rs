use crate::app::navigation_state::ThemeMode;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

pub fn render_appearance_settings_section(ui: &mut egui::Ui, theme_mode: &mut ThemeMode) -> bool {
    let mut changed = false;
    let dark_mode = ui.visuals().dark_mode;

    ui.label(
        RichText::new(t!("settings.appearance"))
            .size(14.0)
            .strong()
            .color(theme::text_color(dark_mode)),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(t!("settings.appearance_description"))
            .size(13.0)
            .color(theme::secondary_text_color(dark_mode)),
    );
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
    ui.label(
        RichText::new(t!("settings.theme_apply_immediately"))
            .size(12.0)
            .color(theme::secondary_text_color(dark_mode)),
    );
    ui.add_space(8.0);
    ui.separator();

    changed
}
