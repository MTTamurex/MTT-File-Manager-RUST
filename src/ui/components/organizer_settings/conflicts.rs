use crate::app::ImageViewerApp;
use crate::infrastructure::organizer::OrganizerConflictResolution;
use crate::ui::components::settings_ui;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

pub(super) fn render_conflicts(ui: &mut egui::Ui, app: &mut ImageViewerApp, dark_mode: bool) {
    ui.add_space(18.0);
    settings_ui::card_frame(dark_mode).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            settings_ui::sub_header(ui, &t!("organizer.conflicts_title"));
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    app.organizer_state
                        .conflict_state
                        .conflicts
                        .len()
                        .to_string(),
                )
                .small()
                .color(theme::secondary_text_color(dark_mode)),
            );
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new(t!("organizer.conflicts_description"))
                .size(13.0)
                .color(theme::secondary_text_color(dark_mode)),
        );
        ui.add_space(12.0);

        if let Some(error) = app.organizer_state.conflict_state.load_error.clone() {
            ui.label(
                RichText::new(t!("organizer.conflicts_load_failed", reason = error))
                    .color(theme::secondary_text_color(dark_mode)),
            );
            if ui.button(t!("organizer.retry_conflicts")).clicked() {
                if let Err(error) = app.organizer_state.reload_conflicts(&app.app_state_db) {
                    app.notifications.warning(error.to_string());
                }
            }
            return;
        }

        if app.organizer_state.conflict_state.conflicts.is_empty() {
            ui.label(
                RichText::new(t!("organizer.no_conflicts"))
                    .color(theme::secondary_text_color(dark_mode)),
            );
            return;
        }

        let conflicts = app.organizer_state.conflict_state.conflicts.clone();
        for conflict in conflicts {
            render_conflict_row(ui, app, dark_mode, conflict);
            ui.add_space(8.0);
        }
    });
    render_destination_confirmation(app, ui.ctx());
}

fn render_conflict_row(
    ui: &mut egui::Ui,
    app: &mut ImageViewerApp,
    dark_mode: bool,
    conflict: crate::infrastructure::app_state_db::OrganizerConflictRecord,
) {
    let conflict_id = conflict.conflict_id;
    let mut source_name = app
        .organizer_state
        .conflict_state
        .source_inputs
        .get(&conflict_id)
        .cloned()
        .unwrap_or_default();
    let mut destination_name = app
        .organizer_state
        .conflict_state
        .destination_inputs
        .get(&conflict_id)
        .cloned()
        .unwrap_or_default();
    let pending = app.organizer_state.is_conflict_command_pending(conflict_id);
    let rename_available = conflict.destination_snapshot.is_some();
    let mut rename_source = false;
    let mut cancel = false;

    settings_ui::row_frame(dark_mode).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            RichText::new(t!("organizer.conflict_id", id = conflict_id.get()))
                .strong()
                .color(theme::text_color(dark_mode)),
        );
        ui.add_space(6.0);
        path_row(
            ui,
            dark_mode,
            &t!("organizer.conflict_source"),
            &conflict.source_path,
        );
        path_row(
            ui,
            dark_mode,
            &t!("organizer.conflict_destination"),
            &conflict.destination_path,
        );
        ui.add_space(8.0);

        ui.add_enabled_ui(!pending, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(t!("organizer.conflict_source_name"))
                        .small()
                        .color(theme::secondary_text_color(dark_mode)),
                );
                settings_ui::text_edit(
                    ui,
                    (ui.available_width() - 4.0).clamp(120.0, 260.0),
                    &mut source_name,
                );
            });
        });
        ui.add_space(5.0);
        ui.add_enabled_ui(!pending, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(t!("organizer.conflict_destination_name"))
                        .small()
                        .color(theme::secondary_text_color(dark_mode)),
                );
                settings_ui::text_edit(
                    ui,
                    (ui.available_width() - 4.0).clamp(120.0, 260.0),
                    &mut destination_name,
                );
            });
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !pending && rename_available,
                    egui::Button::new(t!("organizer.rename_source")),
                )
                .clicked()
            {
                rename_source = true;
            }
            if ui
                .add_enabled(
                    !pending && rename_available,
                    egui::Button::new(t!("organizer.rename_destination")),
                )
                .on_hover_text(t!("organizer.rename_destination_warning"))
                .clicked()
            {
                app.organizer_state.conflict_state.destination_confirmation = Some(conflict_id);
            }
            if ui
                .add_enabled(!pending, egui::Button::new(t!("organizer.cancel_conflict")))
                .clicked()
            {
                cancel = true;
            }
            if pending {
                ui.spinner();
                ui.label(t!("organizer.conflict_resolving"));
            }
        });
        if !rename_available {
            ui.label(
                RichText::new(t!("organizer.conflict_identity_unavailable"))
                    .small()
                    .color(theme::secondary_text_color(dark_mode)),
            );
        }
        ui.label(
            RichText::new(t!("organizer.conflict_keep_hint"))
                .small()
                .italics()
                .color(theme::secondary_text_color(dark_mode)),
        );
    });

    app.organizer_state
        .conflict_state
        .source_inputs
        .insert(conflict_id, source_name.clone());
    app.organizer_state
        .conflict_state
        .destination_inputs
        .insert(conflict_id, destination_name.clone());

    let resolution = if rename_source {
        Some(OrganizerConflictResolution::RenameSource {
            new_name: source_name,
        })
    } else if cancel {
        Some(OrganizerConflictResolution::Cancel)
    } else {
        None
    };
    if let Some(resolution) = resolution {
        if let Err(error) = app
            .organizer_state
            .resolve_conflict(conflict_id, resolution)
        {
            app.notifications.warning(error.to_string());
        }
    }
}

fn render_destination_confirmation(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let Some(conflict_id) = app.organizer_state.conflict_state.destination_confirmation else {
        return;
    };
    let Some(conflict) = app
        .organizer_state
        .conflict_state
        .conflicts
        .iter()
        .find(|conflict| conflict.conflict_id == conflict_id)
        .cloned()
    else {
        app.organizer_state.conflict_state.destination_confirmation = None;
        return;
    };
    let new_name = app
        .organizer_state
        .conflict_state
        .destination_inputs
        .get(&conflict_id)
        .cloned()
        .unwrap_or_default();
    let new_path = conflict
        .destination_path
        .parent()
        .map(|parent| parent.join(&new_name))
        .unwrap_or_default();
    let mut confirm = false;
    let mut cancel = false;
    egui::Window::new(t!("organizer.rename_destination_confirm_title"))
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(t!("organizer.rename_destination_warning"));
            ui.add_space(6.0);
            ui.label(conflict.destination_path.display().to_string());
            ui.label("->");
            ui.label(new_path.display().to_string());
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .button(t!("organizer.rename_destination_confirm"))
                    .clicked()
                {
                    confirm = true;
                }
                if ui.button(t!("organizer.cancel")).clicked() {
                    cancel = true;
                }
            });
        });
    if confirm {
        app.organizer_state.conflict_state.destination_confirmation = None;
        if let Err(error) = app.organizer_state.resolve_conflict(
            conflict_id,
            OrganizerConflictResolution::RenameDestination { new_name },
        ) {
            app.notifications.warning(error.to_string());
        }
    } else if cancel {
        app.organizer_state.conflict_state.destination_confirmation = None;
    }
}

fn path_row(ui: &mut egui::Ui, dark_mode: bool, label: &str, path: &std::path::Path) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(label)
                .small()
                .color(theme::secondary_text_color(dark_mode)),
        );
        ui.label(
            RichText::new(path.display().to_string())
                .small()
                .monospace()
                .color(theme::text_color(dark_mode)),
        );
    });
}
