use crate::app::ImageViewerApp;
use crate::ui::components::settings_ui;
use eframe::egui;
use rust_i18n::t;

pub(super) fn render(ui: &mut egui::Ui, app: &mut ImageViewerApp) {
    settings_ui::subsection_header(ui, &t!("copy_engine.settings_title"), "");
    if settings_ui::toggle_row(
        ui,
        &t!("copy_engine.enable"),
        &mut app.explorer_preferences.native_copy_engine,
    ) {
        app.file_operation_state.copy_runtime.enabled.store(
            app.explorer_preferences.native_copy_engine,
            std::sync::atomic::Ordering::Relaxed,
        );
        app.save_preferences();
        app.save_preferences_async();
    }
    ui.label(t!("copy_engine.description"));
    ui.label(t!("copy_engine.limitations"));
}
