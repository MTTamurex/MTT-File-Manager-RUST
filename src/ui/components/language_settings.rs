//! Language settings modal for switching the application language

use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

/// Available languages with their locale codes and display names
const LANGUAGES: &[(&str, &str)] = &[("pt-BR", "Português (Brasil)"), ("en", "English")];

pub fn render_language_settings_section(ui: &mut egui::Ui) -> bool {
    let mut language_changed = false;
    let dark_mode = ui.visuals().dark_mode;

    ui.label(
        RichText::new(t!("settings.general"))
            .size(16.0)
            .strong()
            .color(theme::text_color(dark_mode)),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(t!("settings.general_description"))
            .size(13.0)
            .color(theme::secondary_text_color(dark_mode)),
    );
    ui.add_space(16.0);

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
    ui.label(
        RichText::new(t!("settings.language_apply_immediately"))
            .size(12.0)
            .color(theme::secondary_text_color(dark_mode)),
    );
    ui.add_space(8.0);
    ui.separator();

    language_changed
}
