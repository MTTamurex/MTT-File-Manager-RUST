//! Language settings modal for switching the application language

use eframe::egui;
use rust_i18n::t;

/// Available languages with their locale codes and display names
const LANGUAGES: &[(&str, &str)] = &[
    ("pt-BR", "Português (Brasil)"),
    ("en", "English"),
];

/// Render the language settings modal window.
/// Returns whether the modal should remain open.
pub fn render_language_settings(ctx: &egui::Context, show_modal: bool) -> bool {
    let mut keep_open = show_modal;

    let response = egui::Window::new(t!("settings.language_title"))
        .collapsible(false)
        .resizable(false)
        .default_width(320.0)
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.add_space(8.0);

                let current_locale = rust_i18n::locale();

                for &(code, display_name) in LANGUAGES {
                    let is_selected = &*current_locale == code;
                    if ui.selectable_label(is_selected, display_name).clicked() && !is_selected {
                        rust_i18n::set_locale(code);
                    }
                }

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    if ui.button(t!("settings.close")).clicked() {
                        keep_open = false;
                    }
                });
            });
        });

    if response.is_none() {
        return false;
    }

    keep_open
}
