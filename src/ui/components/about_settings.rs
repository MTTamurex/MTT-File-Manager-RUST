use eframe::egui;
use rust_i18n::t;

const APP_NAME: &str = "MTT File Manager";
const REPOSITORY_URL: &str = "https://github.com/MTTamurex/MTT-File-Manager-RUST";

pub fn render_about_settings_section(ui: &mut egui::Ui) {
    ui.heading(t!("settings.about"));
    ui.add_space(8.0);
    ui.label(t!("settings.about_description"));
    ui.add_space(16.0);

    ui.group(|ui| {
        ui.set_width(ui.available_width());

        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(APP_NAME).heading().strong());
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(t!("settings.about_status_beta").to_string())
                    .small()
                    .strong()
                    .color(ui.visuals().warn_fg_color),
            );
        });

        ui.add_space(12.0);

        egui::Grid::new("about_settings_grid")
            .num_columns(2)
            .spacing([16.0, 8.0])
            .show(ui, |ui| {
                ui.strong(t!("settings.about_version"));
                ui.label(env!("CARGO_PKG_VERSION"));
                ui.end_row();

                ui.strong(t!("settings.about_repository"));
                ui.hyperlink_to(REPOSITORY_URL, REPOSITORY_URL);
                ui.end_row();

                ui.strong(t!("settings.about_license"));
                ui.label(t!("settings.about_license_value"));
                ui.end_row();

                ui.strong(t!("settings.about_third_party"));
                ui.label(t!("settings.about_third_party_value"));
                ui.end_row();
            });
    });

    ui.add_space(12.0);
    ui.label(egui::RichText::new(t!("settings.about_notice").to_string()).small());
}