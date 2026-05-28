use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

const APP_NAME: &str = "MTT File Manager";
const REPOSITORY_URL: &str = "https://github.com/MTTamurex/MTT-File-Manager-RUST";

pub fn render_about_settings_section(ui: &mut egui::Ui) {
    let dark_mode = ui.visuals().dark_mode;

    ui.label(
        RichText::new(t!("settings.about"))
            .size(16.0)
            .strong()
            .color(theme::text_color(dark_mode)),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(t!("settings.about_description"))
            .size(13.0)
            .color(theme::secondary_text_color(dark_mode)),
    );
    ui.add_space(16.0);

    ui.group(|ui| {
        ui.set_width(ui.available_width());

        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(APP_NAME)
                    .size(16.0)
                    .strong()
                    .color(theme::text_color(dark_mode)),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new(t!("settings.about_status_beta"))
                    .size(12.0)
                    .strong()
                    .color(ui.visuals().warn_fg_color),
            );
        });

        ui.add_space(12.0);

        egui::Grid::new("about_settings_grid")
            .num_columns(2)
            .spacing([16.0, 8.0])
            .show(ui, |ui| {
                ui.label(
                    RichText::new(t!("settings.about_version"))
                        .strong()
                        .color(theme::text_color(dark_mode)),
                );
                ui.label(
                    RichText::new(env!("CARGO_PKG_VERSION")).color(theme::text_color(dark_mode)),
                );
                ui.end_row();

                ui.label(
                    RichText::new(t!("settings.about_repository"))
                        .strong()
                        .color(theme::text_color(dark_mode)),
                );
                ui.hyperlink_to(REPOSITORY_URL, REPOSITORY_URL);
                ui.end_row();

                ui.label(
                    RichText::new(t!("settings.about_license"))
                        .strong()
                        .color(theme::text_color(dark_mode)),
                );
                ui.label(
                    RichText::new(t!("settings.about_license_value"))
                        .color(theme::text_color(dark_mode)),
                );
                ui.end_row();

                ui.label(
                    RichText::new(t!("settings.about_third_party"))
                        .strong()
                        .color(theme::text_color(dark_mode)),
                );
                ui.label(
                    RichText::new(t!("settings.about_third_party_value"))
                        .color(theme::text_color(dark_mode)),
                );
                ui.end_row();
            });
    });

    ui.add_space(12.0);
    ui.label(
        RichText::new(t!("settings.about_notice"))
            .size(12.0)
            .color(theme::secondary_text_color(dark_mode)),
    );
}
