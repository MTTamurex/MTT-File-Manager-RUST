use crate::app::explorer_preferences::ListColumnSizeMode;
use crate::app::ImageViewerApp;
use crate::ui::components::settings_ui;
use eframe::egui;
use rust_i18n::t;

/// One of the three mutually exclusive column sizing mode rows. Clicking an active
/// switch is rejected: exactly one mode stays selected at all times.
fn mode_row(
    ui: &mut egui::Ui,
    label: &str,
    mode: &mut ListColumnSizeMode,
    target: ListColumnSizeMode,
) -> bool {
    let active = *mode == target;
    let mut value = active;
    let clicked = settings_ui::toggle_row(ui, label, &mut value);
    if !clicked {
        return false;
    }
    if active {
        ui.ctx().request_repaint();
        return false;
    }
    *mode = target;
    true
}

pub(super) fn render(ui: &mut egui::Ui, app: &mut ImageViewerApp) {
    settings_ui::subsection_header(ui, &t!("settings.file_browsing"), "");
    let mut mode = app.explorer_preferences.column_size_mode;
    let mut mode_changed = mode_row(
        ui,
        &t!("settings.auto_resize_columns"),
        &mut mode,
        ListColumnSizeMode::AutoFit,
    );
    ui.label(t!("settings.auto_resize_columns_description"));
    ui.add_space(ui.text_style_height(&egui::TextStyle::Body));
    mode_changed |= mode_row(
        ui,
        &t!("settings.fit_columns_to_panel"),
        &mut mode,
        ListColumnSizeMode::FitToPanel,
    );
    ui.label(t!("settings.fit_columns_to_panel_description"));
    ui.add_space(ui.text_style_height(&egui::TextStyle::Body));
    mode_changed |= mode_row(
        ui,
        &t!("settings.remember_column_widths"),
        &mut mode,
        ListColumnSizeMode::RememberPerFolder,
    );
    ui.label(t!("settings.remember_column_widths_description"));
    ui.add_space(10.0);
    let preview_changed = settings_ui::toggle_row(
        ui,
        &t!("settings.hover_preview"),
        &mut app.explorer_preferences.hover_preview,
    );
    ui.label(t!("settings.hover_preview_description"));
    ui.add_space(10.0);
    let sidebar_changed = settings_ui::toggle_row(
        ui,
        &t!("settings.sidebar_unified_scroll"),
        &mut app.explorer_preferences.sidebar_unified_scroll,
    );
    ui.label(t!("settings.sidebar_unified_scroll_description"));
    if sidebar_changed {
        ui.ctx().request_repaint();
    }
    if mode_changed && !matches!(mode, ListColumnSizeMode::RememberPerFolder) {
        app.pending_list_column_autofit = true;
        if let Some(panel) = app.dual_panel_inactive_state.as_mut() {
            panel.pending_list_column_autofit = true;
        }
    }
    if mode_changed || preview_changed || sidebar_changed {
        app.explorer_preferences.column_size_mode = mode;
        app.save_preferences();
        app.save_preferences_async();
    }
}
