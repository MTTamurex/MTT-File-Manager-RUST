use crate::app::ImageViewerApp;
use crate::domain::auto_tag_rule::AutoTagRule;
use crate::domain::file_tag::normalize_tag_path_key;
use crate::infrastructure::auto_tag::AutoTagRuleStatus;
use crate::ui::components::tag_settings::form;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;
use std::path::Path;

pub(crate) fn render_add_rule_form(
    app: &mut ImageViewerApp,
    ui: &mut egui::Ui,
    cancelled: &mut bool,
) -> Option<i64> {
    form::heading(ui, &t!("tags.auto_add"), None);
    ui.label(
        RichText::new(t!("tags.auto_section_description").to_string())
            .size(12.0)
            .color(theme::secondary_text_color(ui.ctx())),
    );
    ui.add_space(12.0);

    let tag_options = crate::ui::components::tag_settings::tag_options(app);

    // Line 1: folder picker + selected folder.
    ui.vertical(|ui| {
        if ui
            .button(t!("tags.auto_pick_folder"))
            .on_hover_text(t!("tags.auto_pick_folder_tooltip"))
            .clicked()
        {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                app.auto_tag_new_folder = Some(path);
            }
        }

        let folder_text = app
            .auto_tag_new_folder
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_else(|| t!("tags.auto_no_folder").to_string());
        ui.add(
            egui::Label::new(
                RichText::new(folder_text.clone())
                    .size(12.0)
                    .color(theme::secondary_text_color(ui.ctx())),
            )
            .wrap(),
        )
        .on_hover_text(folder_text);
    });

    ui.add_space(12.0);

    ui.vertical(|ui| {
        let mut include_subfolders = app.auto_tag_new_include_subfolders;
        if ui
            .checkbox(&mut include_subfolders, t!("tags.auto_include_subfolders"))
            .on_hover_text(t!("tags.auto_include_subfolders_tooltip"))
            .changed()
        {
            app.auto_tag_new_include_subfolders = include_subfolders;
        }

        let selected = app.auto_tag_new_tag;
        form::field(ui, &t!("tags.section"), |ui| {
            render_tag_combo(
                ui,
                ("auto_tag_new_tag", ui.id()),
                &tag_options,
                selected,
                &t!("tags.auto_no_tag"),
                |new| app.auto_tag_new_tag = new,
            );
        });
    });

    if form::creation_actions(ui, &t!("tags.auto_add"), cancelled) {
        return add_rule_from_draft(app);
    }
    ui.add_space(14.0);
    None
}

pub(crate) fn rule_status(app: &ImageViewerApp, rule: &AutoTagRule) -> String {
    if !rule.enabled {
        t!("tags.auto_status_disabled").to_string()
    } else {
        match app
            .auto_tag_status
            .get(&rule.id)
            .copied()
            .unwrap_or(AutoTagRuleStatus::Watching)
        {
            AutoTagRuleStatus::Watching => t!("tags.auto_status_watching").to_string(),
            AutoTagRuleStatus::FolderMissing => t!("tags.auto_status_folder_missing").to_string(),
        }
    }
}

pub(crate) fn render_rule_details(app: &mut ImageViewerApp, ui: &mut egui::Ui, rule_id: i64) {
    let Some(rule) = app
        .auto_tag_rules
        .iter()
        .find(|rule| rule.id == rule_id)
        .cloned()
    else {
        return;
    };
    let tag_options = crate::ui::components::tag_settings::tag_options(app);
    let status_text = rule_status(app, &rule);
    let mut actions = RuleRowActions::default();
    ui.push_id(("rule_details", rule_id), |ui| {
        form::card(ui, |ui| {
            form::heading(ui, &rule.folder_path.to_string_lossy(), Some(&status_text));
            let mut enabled = rule.enabled;
            if ui
                .checkbox(&mut enabled, t!("tags.auto_enabled_tooltip"))
                .on_hover_text(t!("tags.auto_enabled_tooltip"))
                .changed()
            {
                actions.set_enabled = Some(enabled);
            }

            let mut include_subfolders = rule.include_subfolders;
            if ui
                .checkbox(
                    &mut include_subfolders,
                    RichText::new(t!("tags.auto_include_subfolders")).size(11.0),
                )
                .on_hover_text(t!("tags.auto_include_subfolders_tooltip"))
                .changed()
            {
                actions.set_include_subfolders = Some(include_subfolders);
            }

            ui.add_space(8.0);
            form::field(ui, &t!("tags.section"), |ui| {
                render_tag_combo(
                    ui,
                    ("auto_tag_rule_tag", rule.id),
                    &tag_options,
                    Some(rule.tag_id),
                    "",
                    |new| {
                        if new.is_some_and(|tag_id| tag_id != rule.tag_id) {
                            actions.set_tag = new;
                        }
                    },
                );
            });
            ui.separator();
            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(t!("tags.auto_apply_now"))
                    .on_hover_text(t!("tags.auto_apply_now_tooltip"))
                    .clicked()
                {
                    actions.apply_now = true;
                }
                if ui.button(t!("tags.delete")).clicked() {
                    app.auto_tag_delete_confirm = Some(rule.id);
                }
            });

            if app.auto_tag_delete_confirm == Some(rule.id) {
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(t!("tags.auto_delete_confirm").to_string())
                            .color(ui.visuals().warn_fg_color),
                    )
                    .wrap(),
                );
                ui.horizontal_wrapped(|ui| {
                    if ui.button(t!("tags.confirm")).clicked() {
                        actions.delete = true;
                    }
                    if ui.button(t!("tags.cancel")).clicked() {
                        app.auto_tag_delete_confirm = None;
                    }
                });
            }
        });
    });

    if actions
        .set_enabled
        .is_some_and(|enabled| enabled != rule.enabled)
    {
        if let Some(enabled) = actions.set_enabled {
            if app.app_state_db.set_auto_tag_rule_enabled(rule.id, enabled) {
                app.reload_auto_tag_rules();
            }
        }
    }
    if let Some(include) = actions.set_include_subfolders {
        if include != rule.include_subfolders
            && app
                .app_state_db
                .set_auto_tag_rule_include_subfolders(rule.id, include)
        {
            app.reload_auto_tag_rules();
        }
    }
    if let Some(tag_id) = actions.set_tag {
        if folder_tag_pair_is_unique(app, &rule.folder_path, tag_id, Some(rule.id)) {
            if app.app_state_db.update_auto_tag_rule(
                rule.id,
                &rule.folder_path,
                tag_id,
                actions
                    .set_include_subfolders
                    .unwrap_or(rule.include_subfolders),
            ) {
                app.reload_auto_tag_rules();
            }
        } else {
            app.notifications
                .warning(t!("tags.auto_error_duplicate").to_string());
        }
    }
    if actions.apply_now {
        app.auto_tag_manager.apply_existing(rule.id);
    }
    if actions.delete && app.app_state_db.delete_auto_tag_rule(rule.id) {
        app.auto_tag_delete_confirm = None;
        app.reload_auto_tag_rules();
    }
    ui.add_space(8.0);
}

fn render_tag_combo(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    options: &[(i64, String)],
    selected: Option<i64>,
    placeholder: &str,
    on_change: impl FnOnce(Option<i64>),
) {
    let selected_text = selected
        .and_then(|id| {
            options
                .iter()
                .find(|(option_id, _)| *option_id == id)
                .map(|(_, name)| name.clone())
        })
        .unwrap_or_else(|| placeholder.to_string());
    let mut new_selection = selected;
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(selected_text)
        .width(ui.available_width())
        .truncate()
        .show_ui(ui, |ui| {
            for (id, name) in options {
                ui.selectable_value(&mut new_selection, Some(*id), name);
            }
        });
    if new_selection != selected {
        on_change(new_selection);
    }
}

fn folder_tag_pair_is_unique(
    app: &ImageViewerApp,
    folder: &Path,
    tag_id: i64,
    ignore_rule_id: Option<i64>,
) -> bool {
    let folder_key = normalize_tag_path_key(folder);
    !app.auto_tag_rules.iter().any(|rule| {
        Some(rule.id) != ignore_rule_id
            && rule.tag_id == tag_id
            && normalize_tag_path_key(&rule.folder_path) == folder_key
    })
}

fn add_rule_from_draft(app: &mut ImageViewerApp) -> Option<i64> {
    let Some(folder) = app.auto_tag_new_folder.clone() else {
        app.notifications
            .warning(t!("tags.auto_error_no_folder").to_string());
        return None;
    };
    let Some(tag_id) = app.auto_tag_new_tag else {
        app.notifications
            .warning(t!("tags.auto_error_no_tag").to_string());
        return None;
    };

    if !folder.is_dir() {
        app.notifications
            .warning(t!("tags.auto_error_folder_missing").to_string());
        return None;
    }
    let folder_text = folder.to_string_lossy().to_string();
    if crate::domain::special_paths::is_virtual_path(&folder_text) {
        app.notifications
            .warning(t!("tags.auto_error_virtual").to_string());
        return None;
    }
    if !folder_tag_pair_is_unique(app, &folder, tag_id, None) {
        app.notifications
            .warning(t!("tags.auto_error_duplicate").to_string());
        return None;
    }

    if let Some(id) =
        app.app_state_db
            .create_auto_tag_rule(&folder, tag_id, app.auto_tag_new_include_subfolders)
    {
        app.auto_tag_new_folder = None;
        app.auto_tag_new_tag = None;
        app.auto_tag_new_include_subfolders = true;
        app.reload_auto_tag_rules();
        return Some(id);
    }
    None
}

#[derive(Default)]
struct RuleRowActions {
    set_enabled: Option<bool>,
    set_include_subfolders: Option<bool>,
    set_tag: Option<i64>,
    apply_now: bool,
    delete: bool,
}
