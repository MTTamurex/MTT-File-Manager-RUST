use crate::app::ImageViewerApp;
use crate::ui::components::settings_ui;
use eframe::egui;
use rust_i18n::t;

pub(super) fn render(ui: &mut egui::Ui, app: &mut ImageViewerApp) {
    settings_ui::subsection_header(ui, &t!("copy_engine.settings_title"), "");
    let engine_changed = settings_ui::toggle_row(
        ui,
        &t!("copy_engine.enable"),
        &mut app.explorer_preferences.native_copy_engine,
    );
    if engine_changed {
        app.file_operation_state.copy_runtime.enabled.store(
            app.explorer_preferences.native_copy_engine,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
    ui.label(t!("copy_engine.description"));
    ui.label(t!("copy_engine.limitations"));
    ui.add_space(10.0);
    let auto_dismiss_changed = settings_ui::toggle_row(
        ui,
        &t!("copy_engine.auto_dismiss_successful_transfers"),
        &mut app.explorer_preferences.auto_dismiss_successful_transfers,
    );
    ui.label(t!(
        "copy_engine.auto_dismiss_successful_transfers_description"
    ));
    ui.add_space(10.0);
    let engine_on = app.explorer_preferences.native_copy_engine;
    let (confirm_changed, deletions_changed) = ui
        .add_enabled_ui(engine_on, |ui| {
            let confirm_changed = settings_ui::toggle_row(
                ui,
                &t!("copy_engine.confirm_recycle_delete"),
                &mut app.explorer_preferences.confirm_recycle_delete,
            );
            ui.label(t!("copy_engine.confirm_recycle_delete_description"));
            ui.add_space(10.0);
            let deletions_changed = settings_ui::toggle_row(
                ui,
                &t!("copy_engine.auto_dismiss_successful_deletions"),
                &mut app.explorer_preferences.auto_dismiss_successful_deletions,
            );
            ui.label(t!(
                "copy_engine.auto_dismiss_successful_deletions_description"
            ));
            (confirm_changed, deletions_changed)
        })
        .inner;
    if confirm_changed {
        app.file_operation_state
            .copy_runtime
            .skip_recycle_confirmation
            .store(
                !app.explorer_preferences.confirm_recycle_delete,
                std::sync::atomic::Ordering::Relaxed,
            );
    }
    if engine_changed || auto_dismiss_changed || confirm_changed || deletions_changed {
        app.save_preferences();
        app.save_preferences_async();
    }
}
