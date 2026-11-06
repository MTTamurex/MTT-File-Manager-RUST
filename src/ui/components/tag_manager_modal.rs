use crate::app::ImageViewerApp;
use crate::domain::file_tag::TagColor;
use crate::domain::tag_tree;
use crate::ui::components::tag_settings::form;
use eframe::egui::{self, RichText};
use rust_i18n::t;
use rustc_hash::FxHashSet;

enum TagManagerAction {
    Create,
    Rename(i64, String),
    Recolor(i64, TagColor),
    SetParent(i64, Option<i64>),
    Delete(i64),
}

fn color_picker(ui: &mut egui::Ui, id: egui::Id, selected: TagColor) -> Option<TagColor> {
    let color = selected.to_color32();
    let mut rgb = [color.r(), color.g(), color.b()];
    ui.push_id(id, |ui| {
        ui.color_edit_button_srgb(&mut rgb)
            .changed()
            .then_some(TagColor::Custom(rgb))
    })
    .inner
}

/// Parent candidates for a tag: every tag except the tag itself and its own
/// subtree (which would create a cycle).
fn parent_candidates(app: &ImageViewerApp, tag_id: i64) -> Vec<crate::domain::file_tag::FileTag> {
    let excluded: FxHashSet<i64> = tag_tree::subtree_ids(&app.tag_definitions, tag_id)
        .into_iter()
        .collect();
    crate::ui::components::tag_settings::tag_options(app)
        .into_iter()
        .filter(|(id, _)| !excluded.contains(id))
        .filter_map(|(id, label)| {
            app.tag_definitions.get(&id).cloned().map(|mut tag| {
                tag.name = label;
                tag
            })
        })
        .collect()
}

fn parent_combo(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    candidates: &[crate::domain::file_tag::FileTag],
    selected: &mut Option<i64>,
) {
    let selected_text = match selected.and_then(|id| {
        candidates
            .iter()
            .find(|candidate| candidate.id == id)
            .map(|candidate| candidate.name.clone())
    }) {
        Some(name) => name,
        None => t!("tags.parent_root").to_string(),
    };
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(selected_text)
        .width(ui.available_width())
        .truncate()
        .show_ui(ui, |ui| {
            ui.selectable_value(selected, None, t!("tags.parent_root"));
            for candidate in candidates {
                ui.selectable_value(selected, Some(candidate.id), candidate.name.clone());
            }
        });
}

pub fn render_tag_manager_modal(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let mut keep_open = app.show_tag_manager;

    egui::Window::new(t!("tags.manager_title"))
        .id(egui::Id::new("tag_manager_window"))
        .open(&mut keep_open)
        .collapsible(false)
        .resizable(true)
        .default_width(820.0)
        .default_height(560.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            crate::ui::components::tag_settings::render_browser(app, ui);
        });

    app.show_tag_manager = keep_open;
    if !keep_open {
        app.tag_manager_delete_confirm = None;
        app.auto_tag_delete_confirm = None;
    }
}

/// Dedicated creation dialog; the selected tag's editor stays untouched.
pub(crate) fn render_tag_create_form(
    app: &mut ImageViewerApp,
    ui: &mut egui::Ui,
    cancelled: &mut bool,
) -> Option<i64> {
    let mut actions = Vec::new();

    ui.vertical(|ui| {
        form::heading(ui, &t!("tags.add"), None);
        form::field(ui, &t!("tags.name"), |ui| {
            form::input(ui, &mut app.tag_manager_new_name, ui.available_width());
        });
        form::field(ui, &t!("tags.color"), |ui| {
            if let Some(color) =
                color_picker(ui, ui.id().with("new_color"), app.tag_manager_new_color)
            {
                app.tag_manager_new_color = color;
            }
        });
        let new_parent_candidates = parent_candidates(app, 0);
        form::field(ui, &t!("tags.parent"), |ui| {
            parent_combo(
                ui,
                ("tag_manager_new_parent", ui.id()),
                &new_parent_candidates,
                &mut app.tag_manager_new_parent,
            );
        });
        if form::creation_actions(ui, &t!("tags.add"), cancelled) {
            actions.push(TagManagerAction::Create);
        }
    });

    process_tag_actions(app, actions)
}

pub(crate) fn render_tag_details(app: &mut ImageViewerApp, ui: &mut egui::Ui, tag_id: i64) {
    let Some(tag) = app.tag_definitions.get(&tag_id).cloned() else {
        return;
    };
    let assignment_count = app.tag_counts.get(&tag.id).copied().unwrap_or(0);
    let parent_candidates = parent_candidates(app, tag.id);
    let mut actions = Vec::new();
    ui.push_id(("tag_details", tag_id), |ui| {
        form::card(ui, |ui| {
            form::heading(
                ui,
                &t!("tags.browser_details"),
                Some(&t!("tags.usage_count", count = assignment_count)),
            );
            let edit_name = app
                .tag_manager_edit_names
                .entry(tag.id)
                .or_insert_with(|| tag.name.clone());

            form::field(ui, &t!("tags.name"), |ui| {
                if form::name_with_save(ui, edit_name) {
                    actions.push(TagManagerAction::Rename(tag.id, edit_name.clone()));
                }
            });
            form::field(ui, &t!("tags.color"), |ui| {
                if let Some(color) = color_picker(ui, ui.id().with("color"), tag.color) {
                    actions.push(TagManagerAction::Recolor(tag.id, color));
                }
            });
            let mut selected_parent = tag.parent_id;
            form::field(ui, &t!("tags.parent"), |ui| {
                parent_combo(
                    ui,
                    ("tag_manager_parent", tag.id),
                    &parent_candidates,
                    &mut selected_parent,
                );
            });
            if selected_parent != tag.parent_id {
                actions.push(TagManagerAction::SetParent(tag.id, selected_parent));
            }
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(12.0);
            if ui.button(t!("tags.delete")).clicked() {
                app.tag_manager_delete_confirm = Some(tag.id);
            }
            if app.tag_manager_delete_confirm == Some(tag.id) {
                let subtree = tag_tree::subtree_ids(&app.tag_definitions, tag.id);
                let subtag_count = subtree.len().saturating_sub(1);
                let confirm_text = if subtag_count > 0 {
                    let removed: FxHashSet<i64> = subtree.iter().copied().collect();
                    let affected_items = app
                        .tag_assignments
                        .values()
                        .filter(|ids| ids.iter().any(|id| removed.contains(id)))
                        .count();
                    RichText::new(
                        t!(
                            "tags.delete_subtree_confirm",
                            name = tag.name.clone(),
                            subtags = subtag_count,
                            items = affected_items
                        )
                        .to_string(),
                    )
                    .color(ui.visuals().warn_fg_color)
                } else {
                    RichText::new(
                        t!(
                            "tags.delete_confirm",
                            name = tag.name.clone(),
                            count = assignment_count
                        )
                        .to_string(),
                    )
                    .color(ui.visuals().warn_fg_color)
                };
                ui.add_space(6.0);
                ui.add(egui::Label::new(confirm_text).wrap());
                ui.horizontal_wrapped(|ui| {
                    if ui.button(t!("tags.confirm")).clicked() {
                        actions.push(TagManagerAction::Delete(tag.id));
                    }
                    if ui.button(t!("tags.cancel")).clicked() {
                        app.tag_manager_delete_confirm = None;
                    }
                });
            }
        });
    });

    process_tag_actions(app, actions);
}

fn process_tag_actions(app: &mut ImageViewerApp, actions: Vec<TagManagerAction>) -> Option<i64> {
    let mut created = None;
    for action in actions {
        match action {
            TagManagerAction::Create => {
                let name = app.tag_manager_new_name.trim().to_string();
                if name.is_empty() {
                    app.notifications
                        .warning(t!("tags.invalid_name").to_string());
                    continue;
                }
                let parent = app.tag_manager_new_parent;
                if let Some(id) = app.create_new_tag(&name, app.tag_manager_new_color, parent) {
                    created = Some(id);
                    app.tag_manager_new_name.clear();
                    app.tag_manager_new_parent = None;
                } else {
                    app.notifications
                        .warning(t!("tags.duplicate_name").to_string());
                }
            }
            TagManagerAction::Rename(tag_id, name) => {
                let name = name.trim().to_string();
                if name.is_empty() {
                    app.notifications
                        .warning(t!("tags.invalid_name").to_string());
                } else if !app.rename_tag_definition(tag_id, &name) {
                    app.notifications
                        .warning(t!("tags.duplicate_name").to_string());
                }
            }
            TagManagerAction::Recolor(tag_id, color) => {
                app.update_tag_definition_color(tag_id, color);
            }
            TagManagerAction::SetParent(tag_id, parent) => {
                app.set_tag_parent(tag_id, parent);
            }
            TagManagerAction::Delete(tag_id) => {
                app.delete_tag_definition(tag_id);
                app.tag_manager_edit_names.remove(&tag_id);
                app.tag_manager_delete_confirm = None;
            }
        }
    }
    created
}
