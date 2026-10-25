use crate::app::ImageViewerApp;
use crate::ui::components::settings_ui;
use eframe::egui;
use rust_i18n::t;

pub(super) fn render(ui: &mut egui::Ui, app: &mut ImageViewerApp) {
    settings_ui::subsection_header(ui, &t!("settings.file_browsing"), "");
    let columns_changed = settings_ui::toggle_row(
        ui,
        &t!("settings.auto_resize_columns"),
        &mut app.explorer_preferences.auto_resize_columns,
    );
    if columns_changed {
        app.explorer_preferences.remember_column_widths =
            !app.explorer_preferences.auto_resize_columns;
    }
    ui.label(t!("settings.auto_resize_columns_description"));
    ui.add_space(ui.text_style_height(&egui::TextStyle::Body));
    let remember_changed = settings_ui::toggle_row(
        ui,
        &t!("settings.remember_column_widths"),
        &mut app.explorer_preferences.remember_column_widths,
    );
    if remember_changed && app.explorer_preferences.remember_column_widths {
        app.explorer_preferences.auto_resize_columns = false;
    }
    ui.label(t!("settings.remember_column_widths_description"));
    ui.add_space(10.0);
    let preview_changed = settings_ui::toggle_row(
        ui,
        &t!("settings.hover_preview"),
        &mut app.explorer_preferences.hover_preview,
    );
    ui.label(t!("settings.hover_preview_description"));
    if columns_changed && app.explorer_preferences.auto_resize_columns {
        app.pending_list_column_autofit = true;
        if let Some(panel) = app.dual_panel_inactive_state.as_mut() {
            panel.pending_list_column_autofit = true;
        }
    }
    if columns_changed || remember_changed || preview_changed {
        app.save_preferences();
        app.save_preferences_async();
    }
}
