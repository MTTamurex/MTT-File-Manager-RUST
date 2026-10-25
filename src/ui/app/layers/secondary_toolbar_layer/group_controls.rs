use super::column_options;
use crate::app::ImageViewerApp;
use crate::domain::file_entry::GroupMode;
use eframe::egui;
use rust_i18n::t;

pub(super) fn render(ui: &mut egui::Ui, app: &mut ImageViewerApp) {
    let unavailable = app.current_folder_locked || app.navigation_state.is_computer_view;
    ui.scope(|ui| {
        if unavailable {
            ui.disable();
        }
        let mut choices = vec![
            (
                GroupMode::None,
                t!("secondary_toolbar.group_none").to_string(),
            ),
            (
                GroupMode::Name,
                t!("secondary_toolbar.group_name").to_string(),
            ),
            (
                GroupMode::Date,
                t!("secondary_toolbar.group_date").to_string(),
            ),
            (
                GroupMode::Type,
                t!("secondary_toolbar.group_type").to_string(),
            ),
            (
                GroupMode::Size,
                t!("secondary_toolbar.group_size").to_string(),
            ),
        ];
        choices.extend(
            column_options::for_app(app)
                .into_iter()
                .map(|(id, label)| (GroupMode::Column(id), label)),
        );
        let selected = choices
            .iter()
            .find(|(mode, _)| *mode == app.group_mode)
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| match app.group_mode {
                GroupMode::Column(id) => column_options::label(id, &app.columns.catalog),
                _ => t!("secondary_toolbar.group_none").to_string(),
            });
        egui::ComboBox::from_id_salt("group_mode_secondary")
            .width(140.0)
            .selected_text(format!(
                "{}: {}",
                t!("secondary_toolbar.group_by"),
                selected
            ))
            .show_ui(ui, |ui| {
                for (mode, label) in choices {
                    if ui.selectable_label(app.group_mode == mode, label).clicked() {
                        app.group_mode = mode;
                        app.group_mode_normal = mode;
                        app.rebuild_group_projection();
                        app.scroll_offset_y = 0.0;
                        app.scroll_offset_x = 0.0;
                        app.save_preferences();
                    }
                }
            });
        let direction = if app.group_descending {
            "\u{2193}"
        } else {
            "\u{2191}"
        };
        if ui
            .add_enabled(
                app.group_mode != GroupMode::None,
                egui::Button::new(direction),
            )
            .on_hover_text(t!("secondary_toolbar.reverse_group_order"))
            .clicked()
        {
            app.group_descending = !app.group_descending;
            app.group_descending_normal = app.group_descending;
            app.rebuild_group_projection();
            app.save_preferences();
        }
    });
}
