use crate::app::ImageViewerApp;
use crate::ui::components::settings_ui;
use rust_i18n::t;

mod browser;
mod creation;
pub(crate) mod form;
mod layout;
mod model;
mod rule_list;
mod rule_row;
mod tag_list;
mod toolbar;
mod tree_row;

pub(crate) use browser::render_browser;

pub(crate) fn tag_options(app: &ImageViewerApp) -> Vec<(i64, String)> {
    let labels = model::tag_labels(&app.tag_definitions);
    app.sorted_tag_ids()
        .iter()
        .filter_map(|id| labels.get(id).map(|label| (*id, label.clone())))
        .collect()
}

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

    render_browser(app, ui);

    TagSettingsOutput { show_tags_changed }
}
