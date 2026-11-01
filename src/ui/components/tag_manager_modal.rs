use crate::app::ImageViewerApp;
use crate::domain::file_tag::TagColor;
use crate::ui::components::settings_ui;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

enum TagManagerAction {
    Create,
    Rename(i64, String),
    Recolor(i64, TagColor),
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
    // Resolve the current definitions in the cached display order, avoiding the
    // per-frame clone + lowercase + sort of the whole definition set.
    let sorted_ids = app.sorted_tag_ids();
    let tags: Vec<crate::domain::file_tag::FileTag> = sorted_ids
        .iter()
        .filter_map(|id| app.tag_definitions.get(id).cloned())
        .collect();
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

        ui.add_space(12.0);

        if tags.is_empty() {
            ui.label(
                RichText::new(t!("tags.no_tags")).color(theme::secondary_text_color(ui.ctx())),
            );
        }

        for tag in &tags {
            let assignment_count = app.tag_counts.get(&tag.id).copied().unwrap_or(0);
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

                    if app.tag_manager_delete_confirm == Some(tag.id) {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.add_space(24.0);
                            ui.label(
                                RichText::new(
                                    t!(
                                        "tags.delete_confirm",
                                        name = tag.name.clone(),
                                        count = assignment_count
                                    )
                                    .to_string(),
                                )
                                .color(ui.visuals().warn_fg_color),
                            );
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

    for action in actions {
        match action {
            TagManagerAction::Create => {
                let name = app.tag_manager_new_name.trim().to_string();
                if name.is_empty() {
                    app.notifications
                        .warning(t!("tags.invalid_name").to_string());
                    continue;
                }
                if app
                    .create_new_tag(&name, app.tag_manager_new_color)
                    .is_some()
                {
                    app.tag_manager_new_name.clear();
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
            TagManagerAction::Delete(tag_id) => {
                app.delete_tag_definition(tag_id);
                app.tag_manager_edit_names.remove(&tag_id);
                app.tag_manager_delete_confirm = None;
            }
        }
    }
}
