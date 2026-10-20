use crate::app::ImageViewerApp;
use eframe::egui;
use rust_i18n::t;
use std::sync::atomic::Ordering;

pub(crate) fn settings(ui: &mut egui::Ui, app: &mut ImageViewerApp) {
    ui.heading(t!("updates.title"));
    for (key, label, value) in [
        (
            "updates_check_on_startup",
            t!("updates.check_startup"),
            &mut app.updates.check_on_startup,
        ),
        (
            "updates_auto_install",
            t!("updates.auto_install"),
            &mut app.updates.auto_install,
        ),
    ] {
        let previous = *value;
        if ui.checkbox(value, label).changed() {
            if app
                .app_state_db
                .set_preference(key, if *value { "true" } else { "false" })
                .is_err()
            {
                *value = previous;
                app.updates.error = Some(t!("updates.save_failed").to_string());
            }
        }
    }
    ui.small(t!("updates.permission_notice"));
    if ui
        .add_enabled(
            !app.updates.checking && !app.updates.downloading && !app.updates.waiting(),
            egui::Button::new(t!("updates.check")),
        )
        .clicked()
    {
        app.updates.check(ui.ctx(), false);
    }
    status(ui, app);
    if app.updates.release.is_some() && ui.button(t!("updates.details")).clicked() {
        app.updates.show_dialog = true;
    }
}

fn status(ui: &mut egui::Ui, app: &ImageViewerApp) {
    if app.updates.checking {
        ui.label(t!("updates.checking"));
    } else if app.updates.downloading {
        let size = app.updates.release.as_ref().map_or(1, |r| r.size);
        let done = app.updates.progress.load(Ordering::Relaxed);
        ui.add(egui::ProgressBar::new(done as f32 / size as f32).show_percentage());
        ui.label(t!("updates.downloading"));
    } else if app.updates.waiting() {
        ui.label(t!("updates.waiting"));
    } else if app.updates.up_to_date {
        ui.label(t!("updates.current"));
    }
    if let Some(error) = &app.updates.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
}

pub(crate) fn dialog(ctx: &egui::Context, app: &mut ImageViewerApp) {
    if !app.updates.show_dialog || app.updates.exiting {
        return;
    }
    egui::Window::new(t!("updates.title"))
        .id(egui::Id::new("update_dialog"))
        .collapsible(false)
        .default_width(520.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            if let Some(release) = &app.updates.release {
                ui.label(t!(
                    "updates.available",
                    version = release.version.to_string()
                ));
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        // Release text is untrusted: display it without executing HTML or embedded links.
                        ui.label(&release.notes);
                    });
            }
            ui.separator();
            ui.small(t!("updates.permission_notice"));
            status(ui, app);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        app.updates.release.is_some()
                            && !app.updates.checking
                            && !app.updates.downloading
                            && !app.updates.waiting(),
                        egui::Button::new(t!("updates.install")),
                    )
                    .clicked()
                {
                    app.updates.download(ctx);
                }
                if ui
                    .add_enabled(
                        !app.updates.downloading,
                        egui::Button::new(t!("updates.later")),
                    )
                    .clicked()
                {
                    app.updates.postpone();
                }
            });
        });
}
