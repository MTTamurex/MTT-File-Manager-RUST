use crate::app::ImageViewerApp;
use crate::domain::organizer_rule::OrganizerConflictPolicy;
use crate::ui::components::settings_ui;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;
use std::path::PathBuf;

pub(super) fn render_conflict_policy(ui: &mut egui::Ui, app: &mut ImageViewerApp, dark_mode: bool) {
    ui.label(
        RichText::new(t!("organizer.conflict_policy"))
            .color(theme::secondary_text_color(dark_mode)),
    );
    ui.vertical(|ui| {
        // Keep the selector clear of a wrapped presets row above it.
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            let selected = policy_label(&app.organizer_state.conflict_policy);
            egui::ComboBox::from_id_salt("organizer_conflict_policy")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(
                            matches!(
                                &app.organizer_state.conflict_policy,
                                OrganizerConflictPolicy::Ask
                            ),
                            t!("organizer.conflict_policy_ask"),
                        )
                        .clicked()
                    {
                        app.organizer_state.conflict_policy = OrganizerConflictPolicy::Ask;
                    }
                    if ui
                        .selectable_label(
                            matches!(
                                &app.organizer_state.conflict_policy,
                                OrganizerConflictPolicy::Skip
                            ),
                            t!("organizer.conflict_policy_skip"),
                        )
                        .clicked()
                    {
                        app.organizer_state.conflict_policy = OrganizerConflictPolicy::Skip;
                    }
                    if ui
                        .selectable_label(
                            matches!(
                                &app.organizer_state.conflict_policy,
                                OrganizerConflictPolicy::AutoRenameSource
                            ),
                            t!("organizer.conflict_policy_auto_rename"),
                        )
                        .clicked()
                    {
                        app.organizer_state.conflict_policy =
                            OrganizerConflictPolicy::AutoRenameSource;
                    }
                    if ui
                        .selectable_label(
                            matches!(
                                &app.organizer_state.conflict_policy,
                                OrganizerConflictPolicy::MoveToConflictFolder(_)
                            ),
                            t!("organizer.conflict_policy_folder"),
                        )
                        .clicked()
                    {
                        app.organizer_state.conflict_policy =
                            OrganizerConflictPolicy::MoveToConflictFolder(PathBuf::from(
                                app.organizer_state.conflict_folder_input.trim(),
                            ));
                    }
                });
            ui.label(
                RichText::new(policy_hint(&app.organizer_state.conflict_policy))
                    .small()
                    .color(theme::secondary_text_color(dark_mode)),
            );
        });
    });
    ui.end_row();
    if matches!(
        &app.organizer_state.conflict_policy,
        OrganizerConflictPolicy::MoveToConflictFolder(_)
    ) {
        ui.label(
            RichText::new(t!("organizer.conflict_folder"))
                .color(theme::secondary_text_color(dark_mode)),
        );
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let edit_width = (ui.available_width() - 130.0).max(120.0);
                settings_ui::text_edit(
                    ui,
                    edit_width,
                    &mut app.organizer_state.conflict_folder_input,
                );
                app.organizer_state.conflict_policy = OrganizerConflictPolicy::MoveToConflictFolder(
                    PathBuf::from(app.organizer_state.conflict_folder_input.trim()),
                );
                ui.add_space(8.0);
                if ui.button(t!("organizer.choose_folder")).clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        let path = path.to_string_lossy().to_string();
                        app.organizer_state.conflict_folder_input = path.clone();
                        app.organizer_state.conflict_policy =
                            OrganizerConflictPolicy::MoveToConflictFolder(PathBuf::from(path));
                    }
                }
            });
            ui.label(
                RichText::new(t!("organizer.conflict_folder_hint"))
                    .small()
                    .color(theme::secondary_text_color(dark_mode)),
            );
        });
        ui.end_row();
    }
}

pub(super) fn form_conflict_policy(app: &ImageViewerApp) -> OrganizerConflictPolicy {
    match &app.organizer_state.conflict_policy {
        OrganizerConflictPolicy::Ask => OrganizerConflictPolicy::Ask,
        OrganizerConflictPolicy::Skip => OrganizerConflictPolicy::Skip,
        OrganizerConflictPolicy::AutoRenameSource => OrganizerConflictPolicy::AutoRenameSource,
        OrganizerConflictPolicy::MoveToConflictFolder(_) => {
            OrganizerConflictPolicy::MoveToConflictFolder(PathBuf::from(
                app.organizer_state.conflict_folder_input.trim(),
            ))
        }
    }
}

pub(super) fn policy_label(policy: &OrganizerConflictPolicy) -> String {
    match policy {
        OrganizerConflictPolicy::Ask => t!("organizer.conflict_policy_ask").to_string(),
        OrganizerConflictPolicy::Skip => t!("organizer.conflict_policy_skip").to_string(),
        OrganizerConflictPolicy::AutoRenameSource => {
            t!("organizer.conflict_policy_auto_rename").to_string()
        }
        OrganizerConflictPolicy::MoveToConflictFolder(_) => {
            t!("organizer.conflict_policy_folder").to_string()
        }
    }
}

fn policy_hint(policy: &OrganizerConflictPolicy) -> String {
    match policy {
        OrganizerConflictPolicy::Ask => t!("organizer.conflict_policy_manual_hint").to_string(),
        _ => t!("organizer.conflict_policy_hint").to_string(),
    }
}
