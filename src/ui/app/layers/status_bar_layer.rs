use crate::app::ImageViewerApp;
use eframe::egui;

pub(crate) fn render_status_bar_layer(app: &mut ImageViewerApp, ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("status_bar")
        .exact_height(24.0)
        .show(ctx, |ui| {
            use crate::ui::status_bar::{render_status_bar, StatusBarAction};
            let action = render_status_bar(
                ui,
                &mut app.is_loading_folder,
                app.total_items,
                &mut app.view_mode,
                &mut app.sort_mode,
                &mut app.sort_descending,
                &mut app.folders_position,
                &app.cache_manager.texture_cache,
                app.frame_time_avg_ms,
                app.frame_time_peak_ms,
                app.fps_avg,
                app.upload_budget_ms,
                app.is_computer_view,
            );
            match action {
                StatusBarAction::SortChanged => {
                    if app.is_computer_view {
                        app.sort_mode_computer = app.sort_mode;
                    } else {
                        app.sort_mode_normal = app.sort_mode;
                    }
                    app.sort_items();
                    app.save_preferences();
                }
                StatusBarAction::OpenVirtualDriveSettings => {
                    app.show_virtual_drive_settings = true;
                }
                _ => {}
            }
        });
}
