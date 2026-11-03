use crate::app::ImageViewerApp;
use crate::ui::components::settings_ui;
use rust_i18n::t;

pub struct TagSettingsOutput {
    pub show_tags_changed: bool,
}

pub fn render_tag_settings_section(
    ui: &mut eframe::egui::Ui,
    app: &mut ImageViewerApp,
) -> TagSettingsOutput {
    settings_ui::section_header(ui, &t!("settings.tags"), &t!("settings.tags_description"));

    let mut show_tags_changed = false;
    if settings_ui::toggle_row(ui, &t!("settings.show_tags_sidebar"), &mut app.show_tags) {
        show_tags_changed = true;
    }
    ui.add_space(16.0);

    crate::ui::components::tag_manager_modal::render_tag_create_form(app, ui);

    ui.add_space(16.0);
    crate::ui::components::auto_tag_settings::render_auto_tag_settings(app, ui);

    ui.add_space(16.0);
    crate::ui::components::tag_manager_modal::render_tag_list(app, ui);

    TagSettingsOutput { show_tags_changed }
}
