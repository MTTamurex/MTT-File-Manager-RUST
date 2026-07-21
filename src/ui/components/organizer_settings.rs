use crate::app::ImageViewerApp;
use crate::domain::organizer_rule::{
    parse_extensions, validate_rule_set, OrganizerExtensionPreset, OrganizerRule,
    OrganizerRuleError,
};
use crate::infrastructure::app_state_db::OrganizerRuleDbError;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;
use std::path::PathBuf;

pub fn render_organizer_settings_section(ui: &mut egui::Ui, app: &mut ImageViewerApp) {
    let dark_mode = ui.visuals().dark_mode;
    ui.label(
        RichText::new(t!("settings.organizer").to_string())
            .size(16.0)
            .strong()
            .color(theme::text_color(dark_mode)),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(t!("organizer.description"))
            .size(13.0)
            .color(theme::secondary_text_color(dark_mode)),
    );
    ui.add_space(12.0);

    render_rule_form(ui, app, dark_mode);
    ui.add_space(16.0);
    ui.separator();
    ui.add_space(12.0);
    render_rules(ui, app, dark_mode);
}

fn render_rule_form(ui: &mut egui::Ui, app: &mut ImageViewerApp, dark_mode: bool) {
    let editing = app.organizer_state.editing_rule_id.is_some();
    ui.label(
        RichText::new(if editing {
            t!("organizer.edit_rule")
        } else {
            t!("organizer.new_rule")
        })
        .strong()
        .color(theme::text_color(dark_mode)),
    );
    ui.add_space(6.0);

    ui.horizontal(|ui| {
        ui.label(t!("organizer.source"));
        ui.text_edit_singleline(&mut app.organizer_state.source_input);
        if ui.button(t!("organizer.choose_folder")).clicked() {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                app.organizer_state.source_input = path.to_string_lossy().to_string();
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label(t!("organizer.destination"));
        ui.text_edit_singleline(&mut app.organizer_state.destination_input);
        if ui.button(t!("organizer.choose_folder")).clicked() {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                app.organizer_state.destination_input = path.to_string_lossy().to_string();
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label(t!("organizer.extensions"));
        ui.text_edit_singleline(&mut app.organizer_state.extensions_input);
        ui.label(
            RichText::new(t!("organizer.extensions_hint"))
                .small()
                .color(theme::secondary_text_color(dark_mode)),
        );
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(t!("organizer.presets"));
        for preset in OrganizerExtensionPreset::ALL {
            if ui.button(preset_label(preset)).clicked() {
                app.organizer_state.extensions_input = preset.extensions().join(", ");
            }
        }
    });
    ui.add_space(6.0);
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

fn render_rules(ui: &mut egui::Ui, app: &mut ImageViewerApp, dark_mode: bool) {
    if app.organizer_state.rules.is_empty() {
        ui.label(
            RichText::new(t!("organizer.no_rules")).color(theme::secondary_text_color(dark_mode)),
        );
        return;
    }

    let rules = app.organizer_state.rules.clone();
    for mut rule in rules {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                if ui
                    .checkbox(&mut rule.enabled, t!("organizer.enabled"))
                    .changed()
                {
                    let message = if rule.enabled {
                        t!("organizer.enabled_message").to_string()
                    } else {
                        t!("organizer.disabled_message").to_string()
                    };
                    persist_rule(app, &rule, message);
                }
                ui.label(RichText::new(rule.extensions.join(", ")).strong());
            });
            ui.label(format!(
                "{} {}",
                t!("organizer.source"),
                rule.source_folder.display()
            ));
            ui.label(format!(
                "{} {}",
                t!("organizer.destination"),
                rule.destination_folder.display()
            ));
            ui.horizontal(|ui| {
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
    app.organizer_state
        .replace_rules(app.app_state_db.get_organizer_rules());
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
