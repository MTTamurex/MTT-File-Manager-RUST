use crate::app::startup_location::StartupLocation;
use crate::app::ImageViewerApp;
use crate::domain::date_format::{date_format, set_date_format, DateFormat};
use crate::ui::components::settings_ui;
use eframe::egui;
use rust_i18n::t;

pub fn render(ui: &mut egui::Ui, app: &mut ImageViewerApp) -> bool {
    let mut changed = false;
    settings_ui::section_header(
        ui,
        &t!("settings.startup_location"),
        &t!("settings.startup_location_description"),
    );
    let choices = [
        (
            StartupLocation::PreviousSession,
            t!("settings.startup_previous_session"),
        ),
        (StartupLocation::Computer, t!("settings.startup_computer")),
        (StartupLocation::Documents, t!("settings.startup_documents")),
        (
            StartupLocation::UserProfile,
            t!("settings.startup_user_profile"),
        ),
        (StartupLocation::Custom, t!("settings.startup_custom")),
    ];
    let selected = choices
        .iter()
        .find(|(value, _)| *value == app.startup_location)
        .unwrap();
    egui::ComboBox::from_id_salt("startup_location")
        .selected_text(selected.1.as_ref())
        .show_ui(ui, |ui| {
            for (value, label) in &choices {
                changed |= ui
                    .selectable_value(&mut app.startup_location, *value, label.as_ref())
                    .changed();
            }
        });
    if app.startup_location == StartupLocation::Custom {
        ui.add_space(6.0);
        ui.label(if app.startup_custom_path.is_empty() {
            t!("settings.startup_no_folder").into_owned()
        } else {
            app.startup_custom_path.clone()
        });
        if ui.button(t!("settings.startup_choose_folder")).clicked() {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                app.startup_custom_path = path.to_string_lossy().into_owned();
                changed = true;
            }
        }
    }
    ui.add_space(20.0);
    settings_ui::section_header(
        ui,
        &t!("settings.date_format"),
        &t!("settings.date_format_description"),
    );
    let old = date_format();
    let formats = [
        DateFormat::DayMonthYear,
        DateFormat::MonthDayYear,
        DateFormat::YearMonthDay,
    ];
    let selected = formats
        .iter()
        .position(|format| *format == old)
        .unwrap_or(0);
    if let Some(index) =
        settings_ui::segmented_choice(ui, &["DD/MM/YYYY", "MM/DD/YYYY", "YYYY-MM-DD"], selected)
    {
        let new = formats[index];
        if new != old {
            crate::ui::global_search_overlay::date_filter::reformat_search_dates(
                &mut app.global_search,
                new,
            );
            set_date_format(new);
            changed = true;
        }
    }
    changed
}
