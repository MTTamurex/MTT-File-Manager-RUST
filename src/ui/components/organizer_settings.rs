use crate::app::organizer_state::OrganizerFolderCreationRequest;
use crate::app::ImageViewerApp;
use crate::domain::organizer_rule::{
    parse_extensions, validate_rule_set, OrganizerExtensionPreset, OrganizerRule,
    OrganizerRuleError,
};
use crate::infrastructure::app_state_db::OrganizerRuleDbError;
use crate::infrastructure::organizer::{OrganizerCommandError, OrganizerRuleStatus};
use crate::ui::components::settings_ui;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;
use std::path::PathBuf;

mod conflicts;
mod status;

use conflicts::render_conflicts;
use status::{render_folder_creation_confirmation, status_label};

pub fn render_organizer_settings_section(ui: &mut egui::Ui, app: &mut ImageViewerApp) {
    let dark_mode = ui.visuals().dark_mode;
    settings_ui::section_header(ui, &t!("settings.organizer"), &t!("organizer.description"));

    let form_rect = render_rule_form(ui, app, dark_mode);
    ui.add_space(16.0);
    let edit_clicked = render_rules(ui, app, dark_mode);
    if edit_clicked {
        // Jump straight to the form so the user sees the fields being edited.
        ui.scroll_to_rect_animation(
            form_rect,
            Some(egui::Align::Min),
            egui::style::ScrollAnimation::none(),
        );
    }
    render_folder_creation_confirmation(app, ui.ctx());
    render_conflicts(ui, app, dark_mode);
}

fn render_rule_form(ui: &mut egui::Ui, app: &mut ImageViewerApp, dark_mode: bool) -> egui::Rect {
    let editing = app.organizer_state.editing_rule_id.is_some();
    settings_ui::card_frame(dark_mode)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            settings_ui::sub_header(
                ui,
                &if editing {
                    t!("organizer.edit_rule")
                } else {
                    t!("organizer.new_rule")
                },
            );
            ui.add_space(10.0);

            egui::Grid::new("organizer_form_grid")
                .num_columns(2)
                .spacing([16.0, 10.0])
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(t!("organizer.source"))
                            .color(theme::secondary_text_color(dark_mode)),
                    );
                    ui.horizontal(|ui| {
                        let edit_width = (ui.available_width() - 130.0).max(120.0);
                        settings_ui::text_edit(
                            ui,
                            edit_width,
                            &mut app.organizer_state.source_input,
                        );
                        ui.add_space(8.0);
                        if ui.button(t!("organizer.choose_folder")).clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                app.organizer_state.source_input =
                                    path.to_string_lossy().to_string();
                            }
                        }
                    });
                    ui.end_row();

                    ui.label(
                        RichText::new(t!("organizer.destination"))
                            .color(theme::secondary_text_color(dark_mode)),
                    );
                    ui.horizontal(|ui| {
                        let edit_width = (ui.available_width() - 130.0).max(120.0);
                        settings_ui::text_edit(
                            ui,
                            edit_width,
                            &mut app.organizer_state.destination_input,
                        );
                        ui.add_space(8.0);
                        if ui.button(t!("organizer.choose_folder")).clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                app.organizer_state.destination_input =
                                    path.to_string_lossy().to_string();
                            }
                        }
                    });
                    ui.end_row();

                    ui.label(
                        RichText::new(t!("organizer.extensions"))
                            .color(theme::secondary_text_color(dark_mode)),
                    );
                    ui.horizontal(|ui| {
                        let edit_width = (ui.available_width() - 240.0).max(80.0);
                        settings_ui::text_edit(
                            ui,
                            edit_width,
                            &mut app.organizer_state.extensions_input,
                        );
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(t!("organizer.extensions_hint"))
                                .small()
                                .color(theme::secondary_text_color(dark_mode)),
                        );
                    });
                    ui.end_row();

                    ui.label(
                        RichText::new(t!("organizer.presets"))
                            .color(theme::secondary_text_color(dark_mode)),
                    );
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                        for preset in OrganizerExtensionPreset::ALL {
                            if ui.button(preset_label(preset)).clicked() {
                                app.organizer_state.extensions_input =
                                    preset.extensions().join(", ");
                            }
                        }
                    });
                    ui.end_row();
                });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui
                    .button(if editing {
                        t!("organizer.save_rule")
                    } else {
                        t!("organizer.add_rule")
                    })
                    .clicked()
                {
                    save_form_rule(app);
                }
                if editing && ui.button(t!("organizer.cancel")).clicked() {
                    app.organizer_state.reset_form();
                }
            });
        })
        .response
        .rect
}

/// Longest prefix of `text` (plus ellipsis) that fits `max_width` when rendered bold.
fn truncate_to_width(ui: &egui::Ui, text: &str, max_width: f32) -> String {
    let measure = |candidate: &str| {
        egui::WidgetText::from(RichText::new(candidate).strong())
            .into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                egui::TextStyle::Body,
            )
            .size()
            .x
    };
    if measure(text) <= max_width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let candidate: String = chars[..mid].iter().collect::<String>() + "…";
        if measure(&candidate) <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let mut out: String = chars[..lo].iter().collect();
    out.push('…');
    out
}

fn preset_label(preset: OrganizerExtensionPreset) -> String {
    match preset {
        OrganizerExtensionPreset::Documents => t!("organizer.preset_documents").to_string(),
        OrganizerExtensionPreset::Images => t!("organizer.preset_images").to_string(),
        OrganizerExtensionPreset::Videos => t!("organizer.preset_videos").to_string(),
        OrganizerExtensionPreset::Audio => t!("organizer.preset_audio").to_string(),
        OrganizerExtensionPreset::Archives => t!("organizer.preset_archives").to_string(),
        OrganizerExtensionPreset::Executables => t!("organizer.preset_executables").to_string(),
    }
}

fn render_rules(ui: &mut egui::Ui, app: &mut ImageViewerApp, dark_mode: bool) -> bool {
    if app.organizer_state.rules.is_empty() {
        ui.label(
            RichText::new(t!("organizer.no_rules")).color(theme::secondary_text_color(dark_mode)),
        );
        return false;
    }

    let rules = app.organizer_state.rules.clone();
    let mut edit_clicked = false;
    for mut rule in rules {
        let status = app.organizer_state.rule_status(rule.id);
        settings_ui::row_frame(dark_mode).show(ui, |ui| {
            let extensions_text = rule.extensions.join(", ");
            ui.horizontal(|ui| {
                // Reserve room for the toggle so long extension lists never overlap it.
                let text_max = (ui.available_width() - 56.0).max(80.0);
                let display = truncate_to_width(ui, &extensions_text, text_max);
                let label_resp = ui.label(
                    RichText::new(display.clone())
                        .strong()
                        .color(theme::text_color(dark_mode)),
                );
                if display != extensions_text {
                    label_resp.on_hover_text(extensions_text.clone());
                }
                ui.label(
                    RichText::new(status_label(status))
                        .small()
                        .color(theme::secondary_text_color(dark_mode)),
                );
                let toggle_label = format!("{}: {}", t!("organizer.enabled"), extensions_text);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let toggle = settings_ui::toggle_switch(ui, &mut rule.enabled, &toggle_label)
                        .on_hover_text(t!("organizer.enabled").to_string());
                    if toggle.clicked() {
                        let message = if rule.enabled {
                            t!("organizer.enabled_message").to_string()
                        } else {
                            t!("organizer.disabled_message").to_string()
                        };
                        persist_rule(app, &rule, message);
                    }
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(t!("organizer.source").to_string())
                        .color(theme::secondary_text_color(dark_mode)),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new(rule.source_folder.display().to_string())
                        .color(theme::text_color(dark_mode)),
                );
            });
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(t!("organizer.destination").to_string())
                        .color(theme::secondary_text_color(dark_mode)),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new(rule.destination_folder.display().to_string())
                        .color(theme::text_color(dark_mode)),
                );
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if rule.enabled && status == OrganizerRuleStatus::Active {
                    if ui.button(t!("organizer.run_now")).clicked() {
                        let result = app.organizer_state.manager.run_rule_now(rule.id);
                        report_command_error(app, result);
                    }
                    if ui.button(t!("organizer.pause")).clicked() {
                        let result = app.organizer_state.manager.pause_rule(rule.id);
                        report_command_error(app, result);
                    }
                } else if rule.enabled
                    && status == OrganizerRuleStatus::Paused
                    && ui.button(t!("organizer.resume")).clicked()
                {
                    let result = app.organizer_state.manager.resume_rule(rule.id);
                    report_command_error(app, result);
                }
                if matches!(
                    status,
                    OrganizerRuleStatus::SourceUnavailable | OrganizerRuleStatus::BothUnavailable
                ) && ui.button(t!("organizer.create_source")).clicked()
                {
                    app.organizer_state.folder_creation_confirmation =
                        Some(OrganizerFolderCreationRequest {
                            rule_id: rule.id,
                            source: true,
                        });
                }
                if matches!(
                    status,
                    OrganizerRuleStatus::DestinationUnavailable
                        | OrganizerRuleStatus::BothUnavailable
                ) && ui.button(t!("organizer.create_destination")).clicked()
                {
                    app.organizer_state.folder_creation_confirmation =
                        Some(OrganizerFolderCreationRequest {
                            rule_id: rule.id,
                            source: false,
                        });
                }
                if rule.enabled && ui.button(t!("organizer.refresh")).clicked() {
                    let result = app.organizer_state.manager.refresh();
                    report_command_error(app, result);
                }
                let previewing = app.organizer_state.is_previewing(rule.id);
                if ui
                    .add_enabled(
                        rule.enabled && !previewing,
                        egui::Button::new(t!("organizer.preview")),
                    )
                    .clicked()
                {
                    if let Err(error) = app
                        .organizer_state
                        .start_preview(rule.clone(), app.ui_ctx.clone())
                    {
                        app.notifications.warning(error);
                    }
                }
                if previewing {
                    ui.spinner();
                }
                if ui.button(t!("organizer.edit")).clicked() {
                    edit_clicked = true;
                    app.organizer_state.source_input =
                        rule.source_folder.to_string_lossy().to_string();
                    app.organizer_state.destination_input =
                        rule.destination_folder.to_string_lossy().to_string();
                    app.organizer_state.extensions_input = rule.extensions.join(", ");
                    app.organizer_state.editing_rule_id = Some(rule.id);
                    app.organizer_state.form_enabled = rule.enabled;
                }
                if ui.button(t!("organizer.delete")).clicked() {
                    match app.app_state_db.delete_organizer_rule(rule.id) {
                        Ok(()) => {
                            if app.organizer_state.editing_rule_id == Some(rule.id) {
                                app.organizer_state.reset_form();
                            }
                            reload_rules(app);
                            app.notifications
                                .success(t!("organizer.deleted").to_string());
                        }
                        Err(error) => app.notifications.warning(db_error_message(error)),
                    }
                }
            });
        });
        ui.add_space(8.0);
    }
    edit_clicked
}

fn save_form_rule(app: &mut ImageViewerApp) {
    let extensions = match parse_extensions(&app.organizer_state.extensions_input) {
        Ok(extensions) => extensions,
        Err(error) => {
            app.notifications.warning(rule_error_message(error));
            return;
        }
    };
    let rule = match OrganizerRule::new(
        app.organizer_state.editing_rule_id.unwrap_or_default(),
        PathBuf::from(app.organizer_state.source_input.trim()),
        PathBuf::from(app.organizer_state.destination_input.trim()),
        extensions,
        app.organizer_state.form_enabled,
    ) {
        Ok(rule) => rule,
        Err(error) => {
            app.notifications.warning(rule_error_message(error));
            return;
        }
    };
    if persist_rule(app, &rule, t!("organizer.saved").to_string()) {
        app.organizer_state.reset_form();
    }
}

fn persist_rule(app: &mut ImageViewerApp, rule: &OrganizerRule, success_message: String) -> bool {
    let mut proposed_rules = app.organizer_state.rules.clone();
    if let Some(existing) = proposed_rules
        .iter_mut()
        .find(|existing| existing.id == rule.id)
    {
        *existing = rule.clone();
    } else {
        proposed_rules.push(rule.clone());
    }
    if let Err(error) = validate_rule_set(&proposed_rules) {
        app.notifications.warning(rule_error_message(error));
        return false;
    }

    match app.app_state_db.save_organizer_rule(rule) {
        Ok(_) => {
            reload_rules(app);
            app.notifications.success(success_message);
            true
        }
        Err(error) => {
            app.notifications.warning(db_error_message(error));
            false
        }
    }
}

fn reload_rules(app: &mut ImageViewerApp) {
    if let Err(error) = app
        .organizer_state
        .replace_rules(app.app_state_db.get_organizer_rules())
    {
        app.notifications.warning(error.to_string());
    }
}

fn report_command_error(
    app: &mut ImageViewerApp,
    result: Result<crate::infrastructure::organizer::OrganizerCommandId, OrganizerCommandError>,
) {
    if let Err(error) = result {
        app.notifications.warning(error.to_string());
    }
}

fn rule_error_message(error: OrganizerRuleError) -> String {
    match error {
        OrganizerRuleError::InvalidExtensions => {
            t!("organizer.error_invalid_extensions").to_string()
        }
        OrganizerRuleError::MissingExtensions => {
            t!("organizer.error_missing_extensions").to_string()
        }
        OrganizerRuleError::RelativeFolder => t!("organizer.error_relative_folder").to_string(),
        OrganizerRuleError::SourceFolderMissing => t!("organizer.error_source_missing").to_string(),
        OrganizerRuleError::DestinationFolderMissing => {
            t!("organizer.error_destination_missing").to_string()
        }
        OrganizerRuleError::SameFolders => t!("organizer.error_same_folders").to_string(),
        OrganizerRuleError::RuleCycle => t!("organizer.error_rule_cycle").to_string(),
    }
}

fn db_error_message(error: OrganizerRuleDbError) -> String {
    match error {
        OrganizerRuleDbError::DatabaseUnavailable => {
            t!("organizer.error_database_unavailable").to_string()
        }
        OrganizerRuleDbError::RuleNotFound => t!("organizer.error_rule_not_found").to_string(),
        OrganizerRuleDbError::Database(reason) => {
            rust_i18n::t!("organizer.error_database", reason = reason).to_string()
        }
    }
}
