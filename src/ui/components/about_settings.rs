use crate::ui::components::settings_ui;
use crate::ui::theme;
use eframe::egui::{self, Align2, Color32, FontId, RichText};
use rust_i18n::t;

const APP_NAME: &str = "MTT File Manager";
const REPOSITORY_URL: &str = "https://github.com/MTTamurex/MTT-File-Manager-RUST";

pub fn render_about_settings_section(ui: &mut egui::Ui) {
    let dark_mode = ui.visuals().dark_mode;

    settings_ui::section_header(ui, &t!("settings.about"), &t!("settings.about_description"));

    settings_ui::card_frame(dark_mode).show(ui, |ui| {
        ui.set_width(ui.available_width());

        ui.horizontal(|ui| {
            ui.label(
                RichText::new(APP_NAME)
                    .size(16.0)
                    .strong()
                    .color(theme::text_color(dark_mode)),
            );
            ui.add_space(8.0);
            let warn = ui.visuals().warn_fg_color;
            let (r, g, b, _) = warn.to_tuple();
            let (pill_rect, _) =
                ui.allocate_exact_size(egui::vec2(46.0, 20.0), egui::Sense::hover());
            ui.painter().rect_filled(
                pill_rect,
                10.0,
                Color32::from_rgba_unmultiplied(r, g, b, 40),
            );
            ui.painter().text(
                pill_rect.center(),
                Align2::CENTER_CENTER,
                t!("settings.about_status_beta").to_string(),
                FontId::proportional(11.0),
                warn,
            );
        });

        ui.add_space(12.0);

        egui::Grid::new("about_settings_grid")
            .num_columns(2)
            .spacing([16.0, 10.0])
            .show(ui, |ui| {
                about_row(ui, dark_mode, &t!("settings.about_version"), |ui| {
                    ui.label(
                        RichText::new(env!("CARGO_PKG_VERSION"))
                            .color(theme::text_color(dark_mode)),
                    );
                });

                about_row(ui, dark_mode, &t!("settings.about_repository"), |ui| {
                    ui.hyperlink_to(REPOSITORY_URL, REPOSITORY_URL);
                });

                about_row(ui, dark_mode, &t!("settings.about_license"), |ui| {
                    ui.label(
                        RichText::new(t!("settings.about_license_value"))
                            .color(theme::text_color(dark_mode)),
                    );
                });

                about_row(ui, dark_mode, &t!("settings.about_third_party"), |ui| {
                    ui.label(
                        RichText::new(t!("settings.about_third_party_value"))
                            .color(theme::text_color(dark_mode)),
                    );
                });
            });
    });

    ui.add_space(12.0);
    ui.label(
        RichText::new(t!("settings.about_notice"))
            .size(12.0)
            .color(theme::secondary_text_color(dark_mode)),
    );
}

fn about_row(
    ui: &mut egui::Ui,
    dark_mode: bool,
    label: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    ui.label(RichText::new(label).color(theme::secondary_text_color(dark_mode)));
    add_contents(ui);
    ui.end_row();
}
