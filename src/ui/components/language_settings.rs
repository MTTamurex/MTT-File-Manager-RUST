//! Language settings modal for switching the application language

use crate::ui::components::settings_ui;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

/// Available languages with their locale codes and display names
const LANGUAGES: &[(&str, &str)] = &[("pt-BR", "Português (Brasil)"), ("en", "English")];

pub fn render_language_settings_section(ui: &mut egui::Ui) -> bool {
    let mut language_changed = false;
    let dark_mode = ui.visuals().dark_mode;

    settings_ui::section_header(
        ui,
        &t!("settings.general"),
        &t!("settings.general_description"),
    );

    ui.label(
        RichText::new(t!("settings.language"))
            .size(14.0)
            .strong()
            .color(theme::text_color(dark_mode)),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(t!("settings.language_description"))
            .size(13.0)
            .color(theme::secondary_text_color(dark_mode)),
    );
    ui.add_space(10.0);

    let current_locale = rust_i18n::locale();
    let selected = LANGUAGES
        .iter()
        .position(|&(code, _)| code == &*current_locale)
        .unwrap_or(0);
    let labels: Vec<&str> = LANGUAGES
        .iter()
        .map(|&(_, display_name)| display_name)
        .collect();
    if let Some(idx) = settings_ui::segmented_choice(ui, &labels, selected) {
        rust_i18n::set_locale(LANGUAGES[idx].0);
        language_changed = true;
    }

    ui.add_space(12.0);
    ui.label(
        RichText::new(t!("settings.language_apply_immediately"))
            .size(12.0)
            .color(theme::secondary_text_color(dark_mode)),
    );

    language_changed
}
