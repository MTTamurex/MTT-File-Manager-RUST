//! Language settings modal for switching the application language

use crate::ui::components::settings_ui;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

/// Available languages with their locale codes and display names
const LANGUAGES: &[(&str, &str)] = &[
    ("pt-BR", "Português (Brasil)"),
    ("en", "English"),
    ("zh-CN", "简体中文"),
    ("ru", "Русский"),
    ("el", "Ελληνικά"),
];

pub fn render_language_settings_section(ui: &mut egui::Ui) -> bool {
    let mut language_changed = false;
    let dark_mode = ui.visuals().dark_mode;

    settings_ui::section_header(
        ui,
        &t!("settings.general"),
        &t!("settings.general_description"),
    );

    settings_ui::section_header(
        ui,
        &t!("settings.language"),
        &t!("settings.language_description"),
    );

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

#[cfg(test)]
mod tests {
    #[test]
    fn chinese_locale_is_embedded_and_translates() {
        assert_eq!(
            rust_i18n::t!("settings.appearance", locale = "zh-CN").as_ref(),
            "外观"
        );
        assert_eq!(
            rust_i18n::t!("settings.theme_system", locale = "zh-CN").as_ref(),
            "跟随系统"
        );
    }

    #[test]
    fn chinese_locale_translates_a_positional_argument() {
        assert_eq!(
            rust_i18n::t!("status_bar.item_many", locale = "zh-CN", count = 3).as_ref(),
            "3 个项目"
        );
    }

    #[test]
    fn russian_locale_is_embedded_and_translates() {
        assert_eq!(
            rust_i18n::t!("settings.appearance", locale = "ru").as_ref(),
            "Внешний вид"
        );
        assert_eq!(
            rust_i18n::t!("settings.theme_system", locale = "ru").as_ref(),
            "Системная"
        );
    }

    #[test]
    fn russian_locale_translates_a_positional_argument() {
        assert_eq!(
            rust_i18n::t!("status_bar.item_many", locale = "ru", count = 3).as_ref(),
            "Элементов: 3"
        );
    }

    #[test]
    fn greek_locale_is_embedded_and_translates() {
        assert_eq!(
            rust_i18n::t!("settings.appearance", locale = "el").as_ref(),
            "Εμφάνιση"
        );
        assert_eq!(
            rust_i18n::t!("settings.theme_system", locale = "el").as_ref(),
            "Σύστημα"
        );
    }

    #[test]
    fn greek_locale_translates_a_positional_argument() {
        assert_eq!(
            rust_i18n::t!("status_bar.item_many", locale = "el", count = 3).as_ref(),
            "3 στοιχεία"
        );
    }
}
