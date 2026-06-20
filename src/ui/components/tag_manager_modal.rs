use crate::app::ImageViewerApp;
use crate::domain::file_tag::TagColor;
use eframe::egui;
use rust_i18n::t;

enum TagManagerAction {
    Create,
    Rename(i64, String),
    Recolor(i64, TagColor),
    Delete(i64),
}

fn color_label(color: TagColor) -> String {
    match color {
        TagColor::Red => t!("tags.color_red").to_string(),
        TagColor::Orange => t!("tags.color_orange").to_string(),
        TagColor::Yellow => t!("tags.color_yellow").to_string(),
        TagColor::Green => t!("tags.color_green").to_string(),
        TagColor::Blue => t!("tags.color_blue").to_string(),
        TagColor::Purple => t!("tags.color_purple").to_string(),
        TagColor::Gray => t!("tags.color_gray").to_string(),
    }
}

fn color_button(ui: &mut egui::Ui, color: TagColor, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        ui.painter()
            .circle_filled(rect.center(), 7.0, color.to_color32());
        if selected || response.hovered() {
            ui.painter().circle_stroke(
                rect.center(),
                9.0,
                egui::Stroke::new(if selected { 2.0 } else { 1.0 }, ui.visuals().text_color()),
            );
        }
    }
    response.on_hover_text(color_label(color))
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
    let tags = app.sorted_tag_definitions();
    let mut actions = Vec::new();

    ui.vertical(|ui| {
        ui.heading(t!("tags.add"));
        ui.horizontal(|ui| {
            ui.label(t!("tags.name"));
            ui.text_edit_singleline(&mut app.tag_manager_new_name);
            for color in TagColor::default_palette() {
                if color_button(ui, color, app.tag_manager_new_color == color).clicked() {
                    app.tag_manager_new_color = color;
                }
            }
            if ui.button(t!("tags.add")).clicked() {
                actions.push(TagManagerAction::Create);
            }
        });

        ui.separator();

        if tags.is_empty() {
            ui.label(t!("tags.no_tags"));
        }

        for tag in &tags {
            let assignment_count = app.tag_counts.get(&tag.id).copied().unwrap_or(0);
            let edit_name = app
                .tag_manager_edit_names
                .entry(tag.id)
                .or_insert_with(|| tag.name.clone());

            ui.horizontal(|ui| {
                ui.painter().circle_filled(
                    egui::pos2(ui.cursor().left() + 7.0, ui.cursor().top() + 11.0),
                    5.0,
                    tag.color.to_color32(),
                );
                ui.add_space(18.0);
                ui.text_edit_singleline(edit_name);
                if ui.button(t!("tags.save")).clicked() {
                    actions.push(TagManagerAction::Rename(tag.id, edit_name.clone()));
                }
                for color in TagColor::default_palette() {
                    if color_button(ui, color, tag.color == color).clicked() {
                        actions.push(TagManagerAction::Recolor(tag.id, color));
                    }
                }
                ui.label(assignment_count.to_string());
                if ui.button(t!("tags.delete")).clicked() {
                    app.tag_manager_delete_confirm = Some(tag.id);
                }
            });

            if app.tag_manager_delete_confirm == Some(tag.id) {
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    ui.label(
                        t!(
                            "tags.delete_confirm",
                            name = tag.name.clone(),
                            count = assignment_count
                        )
                        .to_string(),
                    );
                    if ui.button(t!("tags.confirm")).clicked() {
                        actions.push(TagManagerAction::Delete(tag.id));
                    }
                    if ui.button(t!("tags.cancel")).clicked() {
                        app.tag_manager_delete_confirm = None;
                    }
                });
            }
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
