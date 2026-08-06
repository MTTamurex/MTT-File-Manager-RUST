use crate::app::navigation_state::ThemeMode;
use crate::ui::components::settings_ui;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

pub fn render_appearance_settings_section(ui: &mut egui::Ui, theme_mode: &mut ThemeMode) -> bool {
    let mut changed = false;
    let dark_mode = ui.visuals().dark_mode;

    settings_ui::section_header(
        ui,
        &t!("settings.appearance"),
        &t!("settings.appearance_description"),
    );

    let modes: [(ThemeMode, &str); 2] = [
        (ThemeMode::Light, &t!("settings.theme_light")),
        (ThemeMode::Dark, &t!("settings.theme_dark")),
    ];
    let selected = modes
        .iter()
        .position(|(mode, _)| mode == theme_mode)
        .unwrap_or(0);
    let labels: Vec<&str> = modes.iter().map(|(_, name)| *name).collect();
    if let Some(idx) = settings_ui::segmented_choice(ui, &labels, selected) {
        *theme_mode = modes[idx].0;
        changed = true;
    }

    ui.add_space(12.0);
    ui.label(
        RichText::new(t!("settings.theme_apply_immediately"))
            .size(12.0)
            .color(theme::secondary_text_color(dark_mode)),
    );

    changed
}
