use crate::app::{tab_preferences::NewTabLocation, ImageViewerApp};
use crate::ui::components::settings_ui;
use eframe::egui;
use rust_i18n::t;

pub fn render(ui: &mut egui::Ui, app: &mut ImageViewerApp) -> bool {
    settings_ui::subsection_header(ui, &t!("settings.tabs"), &t!("settings.new_tab_location"));
    let choices = [
        (NewTabLocation::Current, t!("settings.new_tab_current")),
        (NewTabLocation::Computer, t!("settings.startup_computer")),
        (NewTabLocation::Documents, t!("settings.startup_documents")),
        (
            NewTabLocation::UserProfile,
            t!("settings.startup_user_profile"),
        ),
        (NewTabLocation::Custom, t!("settings.startup_custom")),
    ];
    let prefs = &mut app.tab_preferences;
    let mut changed = false;
    let selected = choices
        .iter()
        .find(|(value, _)| *value == prefs.location)
        .unwrap();
    egui::ComboBox::from_id_salt("new_tab_location")
        .selected_text(selected.1.as_ref())
        .show_ui(ui, |ui| {
            for (value, label) in &choices {
                changed |= ui
                    .selectable_value(&mut prefs.location, *value, label.as_ref())
                    .changed();
            }
        });
    if prefs.location == NewTabLocation::Custom {
        ui.add_space(6.0);
        ui.label(if prefs.custom_path.is_empty() {
            t!("settings.startup_no_folder").into_owned()
        } else {
            prefs.custom_path.clone()
        });
        if ui.button(t!("settings.startup_choose_folder")).clicked() {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                prefs.custom_path = path.to_string_lossy().into_owned();
                changed = true;
            }
        }
    }
    ui.add_space(12.0);
    changed |= settings_ui::toggle_row(
        ui,
        &t!("settings.activate_new_tabs"),
        &mut prefs.activate_new_tabs,
    );
    ui.label(
        egui::RichText::new(t!("settings.activate_new_tabs_description"))
            .small()
            .weak(),
    );
    ui.add_space(12.0);
    changed |= settings_ui::toggle_row(
        ui,
        &t!("settings.archives_in_new_tab"),
        &mut prefs.archives_in_new_tab,
    );
    changed
}
