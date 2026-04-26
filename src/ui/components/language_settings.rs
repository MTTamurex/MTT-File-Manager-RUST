//! Language settings modal for switching the application language

use eframe::egui;
use rust_i18n::t;

/// Available languages with their locale codes and display names
const LANGUAGES: &[(&str, &str)] = &[("pt-BR", "Português (Brasil)"), ("en", "English")];

pub fn render_language_settings_section(ui: &mut egui::Ui) -> bool {
    let mut language_changed = false;

    ui.heading(t!("settings.general"));
    ui.add_space(8.0);
    ui.label(t!("settings.general_description"));
    ui.add_space(16.0);

    ui.label(egui::RichText::new(t!("settings.language").to_string()).strong());
    ui.add_space(4.0);
    ui.label(t!("settings.language_description"));
    ui.add_space(12.0);

    let current_locale = rust_i18n::locale();

    for &(code, display_name) in LANGUAGES {
        let is_selected = &*current_locale == code;
        if ui.selectable_label(is_selected, display_name).clicked() && !is_selected {
            rust_i18n::set_locale(code);
            language_changed = true;
        }
    }

    ui.add_space(12.0);
    ui.label(egui::RichText::new(t!("settings.language_apply_immediately").to_string()).small());
    ui.add_space(8.0);
    ui.separator();

    language_changed
}
