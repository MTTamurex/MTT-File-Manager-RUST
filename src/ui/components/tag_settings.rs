use crate::app::ImageViewerApp;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

pub struct TagSettingsOutput {
    pub show_tags_changed: bool,
}

pub fn render_tag_settings_section(
    ui: &mut egui::Ui,
    app: &mut ImageViewerApp,
) -> TagSettingsOutput {
    let dark_mode = ui.visuals().dark_mode;

    ui.label(
        RichText::new(t!("settings.tags").to_string())
            .size(16.0)
            .strong()
            .color(theme::text_color(dark_mode)),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(t!("settings.tags_description"))
            .size(13.0)
            .color(theme::secondary_text_color(dark_mode)),
    );
    ui.add_space(12.0);

    ui.label(
        RichText::new(t!("settings.show_tags_sidebar").to_string())
            .strong()
            .color(theme::text_color(dark_mode)),
    );
    ui.add_space(4.0);
    let mut show_tags_changed = false;
    if ui
        .checkbox(
            &mut app.show_tags,
            RichText::new(t!("settings.show_tags_sidebar"))
                .color(theme::text_color(dark_mode)),
        )
        .changed()
    {
        show_tags_changed = true;
    }
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(12.0);

    crate::ui::components::tag_manager_modal::render_tag_manager_content(app, ui);

    TagSettingsOutput { show_tags_changed }
}
