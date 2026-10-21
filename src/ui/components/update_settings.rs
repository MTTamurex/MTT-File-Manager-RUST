use crate::app::ImageViewerApp;
use crate::ui::{components::settings_ui, theme};
use eframe::egui::{self, RichText};
use rust_i18n::t;
use std::sync::atomic::Ordering;

pub(crate) fn settings(ui: &mut egui::Ui, app: &mut ImageViewerApp) {
    ui.add_space(8.0);
    settings_ui::section_header(ui, &t!("updates.title"), &t!("updates.description"));
    settings_ui::card_frame(ui.ctx()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 10.0;
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
            if preference_row(ui, &label, value).changed()
                && app
                    .app_state_db
                    .set_preference(key, if *value { "true" } else { "false" })
                    .is_err()
            {
                *value = previous;
                app.updates.error = Some(t!("updates.save_failed").to_string());
            }
        }
        ui.label(
            RichText::new(t!("updates.auto_install_hint"))
                .small()
                .color(theme::secondary_text_color(ui.ctx())),
        );
        ui.add_space(2.0);
        ui.separator();
        ui.label(
            RichText::new(t!("updates.permission_notice"))
                .small()
                .color(theme::secondary_text_color(ui.ctx())),
        );
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().button_padding = egui::vec2(12.0, 7.0);
            if ui
                .add_enabled(
                    !app.updates.checking && !app.updates.downloading && !app.updates.waiting(),
                    egui::Button::new(t!("updates.check")),
                )
                .clicked()
            {
                app.updates.check(ui.ctx(), false);
            }
            if app.updates.release.is_some() && ui.button(t!("updates.details")).clicked() {
                app.updates.show_dialog = true;
            }
        });
        status(ui, app);
    });
}

fn preference_row(ui: &mut egui::Ui, label: &str, value: &mut bool) -> egui::Response {
    ui.horizontal(|ui| {
        let label_width = (ui.available_width() - 40.0 - ui.spacing().item_spacing.x).max(1.0);
        let text = ui
            .allocate_ui_with_layout(
                egui::vec2(label_width, 24.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(label).size(13.0))
                            .wrap()
                            .sense(egui::Sense::click()),
                    )
                },
            )
            .inner;
        let mut response = settings_ui::toggle_switch(ui, value, label);
        if text.clicked() {
            *value = !*value;
            response.mark_changed();
        }
        response
    })
    .inner
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
        .default_width(640.0)
        .max_size((ctx.viewport_rect().size() - egui::vec2(32.0, 32.0)).max(egui::vec2(1.0, 1.0)))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(10.0, 12.0);
            ui.spacing_mut().button_padding = egui::vec2(14.0, 7.0);
            if let Some(release) = &app.updates.release {
                ui.label(
                    RichText::new(t!(
                        "updates.available",
                        version = release.version.to_string()
                    ))
                    .strong(),
                );
                let notes_height = (ctx.viewport_rect().height() * 0.45).clamp(80.0, 360.0);
                settings_ui::card_frame(ui.ctx()).show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt(("release_notes", release.version.to_string()))
                        .max_height(notes_height)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            crate::ui::components::release_notes::show(
                                ui,
                                &mut app.updates.notes_cache,
                                &release.notes,
                            );
                        });
                });
            }
            ui.separator();
            ui.label(
                RichText::new(t!("updates.permission_notice"))
                    .small()
                    .color(theme::secondary_text_color(ui.ctx())),
            );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_update_labels_wrap_before_the_switch_in_narrow_settings() {
        let ctx = egui::Context::default();
        let mut switch_rect = egui::Rect::NOTHING;
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(220.0);
            let mut enabled = false;
            switch_rect = preference_row(
                ui,
                "Check for updates when opening MTT with a longer translated label",
                &mut enabled,
            )
            .rect;
        });
        assert!(switch_rect.right() <= 230.0);
        let mut text_shapes = 0;
        for shape in output.shapes {
            if let egui::epaint::Shape::Text(text) = shape.shape {
                text_shapes += 1;
                assert!(text.pos.x + text.galley.size().x <= switch_rect.left());
            }
        }
        assert!(text_shapes > 0);
    }
}
