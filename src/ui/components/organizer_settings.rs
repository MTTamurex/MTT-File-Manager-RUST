use crate::app::ImageViewerApp;
use crate::domain::organizer_rule::{parse_extensions, preview_rule, OrganizerRule};
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
                    persist_rule(app, &rule);
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
                if ui
                    .add_enabled(rule.enabled, egui::Button::new(t!("organizer.preview")))
                    .clicked()
                {
                    let count = preview_rule(&rule).len();
                    app.notifications
                        .info(format!("{}: {count}", t!("organizer.preview_result")));
                }
                if ui
                    .add_enabled(rule.enabled, egui::Button::new(t!("organizer.run_now")))
                    .clicked()
                {
                    app.organizer_state.manager.run_now(rule.id);
                    app.notifications
                        .info(t!("organizer.run_started").to_string());
                }
                if ui.button(t!("organizer.edit")).clicked() {
                    app.organizer_state.source_input =
                        rule.source_folder.to_string_lossy().to_string();
                    app.organizer_state.destination_input =
                        rule.destination_folder.to_string_lossy().to_string();
                    app.organizer_state.extensions_input = rule.extensions.join(", ");
                    app.organizer_state.editing_rule_id = Some(rule.id);
                }
                if ui.button(t!("organizer.delete")).clicked() {
                    app.app_state_db.delete_organizer_rule(rule.id);
                    reload_rules(app);
                    app.notifications
                        .success(t!("organizer.deleted").to_string());
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
            app.notifications.warning(error);
            return;
        }
    };
    let rule = match OrganizerRule::new(
        app.organizer_state.editing_rule_id.unwrap_or_default(),
        PathBuf::from(app.organizer_state.source_input.trim()),
        PathBuf::from(app.organizer_state.destination_input.trim()),
        extensions,
        true,
    ) {
        Ok(rule) => rule,
        Err(error) => {
            app.notifications.warning(error);
            return;
        }
    };
    persist_rule(app, &rule);
    app.organizer_state.reset_form();
}

fn persist_rule(app: &mut ImageViewerApp, rule: &OrganizerRule) {
    match app.app_state_db.save_organizer_rule(rule) {
        Ok(_) => {
            reload_rules(app);
            app.notifications.success(t!("organizer.saved").to_string());
        }
        Err(error) => app.notifications.warning(error),
    }
}

fn reload_rules(app: &mut ImageViewerApp) {
    app.organizer_state
        .replace_rules(app.app_state_db.get_organizer_rules());
}
