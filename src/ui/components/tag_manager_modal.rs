use crate::app::ImageViewerApp;
use crate::domain::file_tag::TagColor;
use crate::domain::tag_tree;
use crate::ui::components::settings_ui;
use crate::ui::theme;
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
    let mut candidates: Vec<crate::domain::file_tag::FileTag> = app
        .tag_definitions
        .values()
        .filter(|tag| !excluded.contains(&tag.id))
        .cloned()
        .collect();
    candidates.sort_by(|a, b| {
        (a.position, a.name.to_lowercase()).cmp(&(b.position, b.name.to_lowercase()))
    });
    candidates
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
        .width(160.0)
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
        .default_width(520.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            render_tag_manager_content(app, ui);
        });

    app.show_tag_manager = keep_open;
    if !keep_open {
        app.tag_manager_delete_confirm = None;
    }
}

pub fn render_tag_manager_content(app: &mut ImageViewerApp, ui: &mut egui::Ui) {
    render_tag_create_form(app, ui);
    ui.add_space(12.0);
    render_tag_list(app, ui);
}

/// The "Add tag" form, shown in the settings section before the auto-tag
/// block and inside the manager modal.
pub fn render_tag_create_form(app: &mut ImageViewerApp, ui: &mut egui::Ui) {
    let mut actions = Vec::new();

    ui.vertical(|ui| {
        settings_ui::sub_header(ui, &t!("tags.add"));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(t!("tags.name")).color(theme::secondary_text_color(ui.ctx())));
            let edit_width = (ui.available_width() - 170.0).max(120.0);
            settings_ui::text_edit(ui, edit_width, &mut app.tag_manager_new_name);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(t!("tags.add")).clicked() {
                    actions.push(TagManagerAction::Create);
                }
                if let Some(color) = color_picker(
                    ui,
                    egui::Id::new("tag_manager_new_color_picker"),
                    app.tag_manager_new_color,
                ) {
                    app.tag_manager_new_color = color;
                }
            });
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new(t!("tags.parent")).color(theme::secondary_text_color(ui.ctx())));
            let new_parent_candidates = parent_candidates(app, 0);
            parent_combo(
                ui,
                ("tag_manager_new_parent", ui.id()),
                &new_parent_candidates,
                &mut app.tag_manager_new_parent,
            );
        });
    });

    process_tag_actions(app, actions);
}

/// The tag list with edit/recolor/reparent/delete controls, shown after the
/// create form (and after the auto-tag block in the settings section).
pub fn render_tag_list(app: &mut ImageViewerApp, ui: &mut egui::Ui) {
    // Resolve the current definitions in the cached display order (DFS
    // pre-order), avoiding the per-frame clone + lowercase + sort of the
    // whole definition set.
    let sorted_ids = app.sorted_tag_ids();
    let tags: Vec<crate::domain::file_tag::FileTag> = sorted_ids
        .iter()
        .filter_map(|id| app.tag_definitions.get(id).cloned())
        .collect();
    let mut actions = Vec::new();

    ui.vertical(|ui| {
        if tags.is_empty() {
            ui.label(
                RichText::new(t!("tags.no_tags")).color(theme::secondary_text_color(ui.ctx())),
            );
        }

        for tag in &tags {
            let assignment_count = app.tag_counts.get(&tag.id).copied().unwrap_or(0);
            let depth = tag_tree::tag_depth(&app.tag_definitions, tag.id);
            let indent = depth as f32 * 16.0;
            let parent_candidates = parent_candidates(app, tag.id);
            let edit_name = app
                .tag_manager_edit_names
                .entry(tag.id)
                .or_insert_with(|| tag.name.clone());

            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: 4,
                    right: 4,
                    top: 4,
                    bottom: 4,
                })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(indent);
                        let (icon_rect, icon_resp) =
                            ui.allocate_exact_size(egui::vec2(18.0, 26.0), egui::Sense::hover());
                        crate::ui::tag_icon::paint_filled(
                            ui.painter(),
                            icon_rect.center(),
                            12.0,
                            tag.color.to_color32(),
                        );
                        icon_resp.on_hover_text(
                            t!("tags.usage_count", count = assignment_count).to_string(),
                        );
                        ui.add_space(10.0);
                        let edit_width = (ui.available_width() - 190.0).max(80.0);
                        settings_ui::text_edit(ui, edit_width, edit_name);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(t!("tags.delete")).clicked() {
                                app.tag_manager_delete_confirm = Some(tag.id);
                            }
                            if let Some(color) = color_picker(
                                ui,
                                egui::Id::new(("tag_manager_color_picker", tag.id)),
                                tag.color,
                            ) {
                                actions.push(TagManagerAction::Recolor(tag.id, color));
                            }
                            if ui.button(t!("tags.save")).clicked() {
                                actions.push(TagManagerAction::Rename(tag.id, edit_name.clone()));
                            }
                        });
                    });

                    ui.horizontal(|ui| {
                        ui.add_space(32.0 + indent);
                        ui.label(
                            RichText::new(t!("tags.parent"))
                                .color(theme::secondary_text_color(ui.ctx())),
                        );
                        let mut selected_parent = tag.parent_id;
                        parent_combo(
                            ui,
                            ("tag_manager_parent", tag.id),
                            &parent_candidates,
                            &mut selected_parent,
                        );
                        if selected_parent != tag.parent_id {
                            actions.push(TagManagerAction::SetParent(tag.id, selected_parent));
                        }
                    });

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
                        ui.horizontal(|ui| {
                            ui.add_space(24.0 + indent);
                            ui.label(confirm_text);
                            if ui.button(t!("tags.confirm")).clicked() {
                                actions.push(TagManagerAction::Delete(tag.id));
                            }
                            if ui.button(t!("tags.cancel")).clicked() {
                                app.tag_manager_delete_confirm = None;
                            }
                        });
                    }
                });
            ui.add_space(2.0);
        }
    });

    process_tag_actions(app, actions);
}

fn process_tag_actions(app: &mut ImageViewerApp, actions: Vec<TagManagerAction>) {
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
                if app
                    .create_new_tag(&name, app.tag_manager_new_color, parent)
                    .is_some()
                {
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
}
