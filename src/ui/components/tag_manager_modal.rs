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
        TagColor::Pink => t!("tags.color_pink").to_string(),
        TagColor::Brown => t!("tags.color_brown").to_string(),
        TagColor::Mint => t!("tags.color_mint").to_string(),
        TagColor::Teal => t!("tags.color_teal").to_string(),
        TagColor::Cyan => t!("tags.color_cyan").to_string(),
        TagColor::Indigo => t!("tags.color_indigo").to_string(),
        TagColor::Lime => t!("tags.color_lime").to_string(),
        TagColor::Olive => t!("tags.color_olive").to_string(),
        TagColor::Black => t!("tags.color_black").to_string(),
    }
}

fn color_picker_button(ui: &mut egui::Ui, selected: TagColor) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(42.0, 24.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        ui.painter().rect(
            rect,
            4.0,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        ui.painter().circle_filled(
            egui::pos2(rect.left() + 14.0, rect.center().y),
            6.0,
            selected.to_color32(),
        );
        ui.painter().circle_stroke(
            egui::pos2(rect.left() + 14.0, rect.center().y),
            6.0,
            ui.visuals().widgets.noninteractive.bg_stroke,
        );

        ui.painter().text(
            egui::pos2(rect.right() - 11.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            "v",
            egui::TextStyle::Button.resolve(ui.style()),
            ui.visuals().text_color(),
        );
    }
    response.on_hover_text(color_label(selected))
}

fn color_swatch(ui: &mut egui::Ui, color: TagColor, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let swatch_rect = rect.shrink(3.0);
        ui.painter()
            .rect_filled(swatch_rect, 3.0, color.to_color32());
        ui.painter().rect_stroke(
            swatch_rect,
            3.0,
            ui.visuals().widgets.noninteractive.bg_stroke,
            egui::StrokeKind::Inside,
        );

        if selected || response.hovered() {
            let stroke_width = if selected { 2.0 } else { 1.0 };
            ui.painter().rect_stroke(
                swatch_rect.expand(2.0),
                4.0,
                egui::Stroke::new(stroke_width, ui.visuals().text_color()),
                egui::StrokeKind::Inside,
            );
        }
    }
    response.on_hover_text(color_label(color))
}

fn color_picker(ui: &mut egui::Ui, id: egui::Id, selected: TagColor) -> Option<TagColor> {
    let popup_id = id.with("popup");
    let mut show_popup = ui
        .ctx()
        .memory(|m| m.data.get_temp::<bool>(popup_id).unwrap_or(false));

    let button_response = color_picker_button(ui, selected);
    let button_rect = button_response.rect;

    if button_response.clicked() {
        show_popup = !show_popup;
        ui.ctx()
            .memory_mut(|m| m.data.insert_temp::<bool>(popup_id, show_popup));
    }

    if !show_popup {
        return None;
    }

    let mut chosen = None;
    let mut close_popup = false;
    let popup_pos = egui::pos2(button_rect.left(), button_rect.bottom() + 2.0);
    let popup_response = egui::Area::new(popup_id)
        .order(egui::Order::Foreground)
        .fixed_pos(popup_pos)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.set_min_width(136.0);
                    ui.set_max_width(136.0);
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);

                    egui::Grid::new(id.with("grid"))
                        .num_columns(4)
                        .spacing(egui::vec2(4.0, 4.0))
                        .show(ui, |ui| {
                            for (index, color) in
                                TagColor::expanded_palette().into_iter().enumerate()
                            {
                                if color_swatch(ui, color, selected == color).clicked() {
                                    chosen = Some(color);
                                    close_popup = true;
                                }
                                if (index + 1) % 4 == 0 {
                                    ui.end_row();
                                }
                            }
                        });
                });
        });

    if close_popup {
        ui.ctx()
            .memory_mut(|m| m.data.insert_temp::<bool>(popup_id, false));
        return chosen;
    }

    if ui.ctx().input(|i| i.pointer.any_pressed()) {
        if let Some(pointer_pos) = ui.ctx().input(|i| i.pointer.press_origin()) {
            let clicked_button = button_rect.contains(pointer_pos);
            let clicked_popup = popup_response.response.rect.contains(pointer_pos);
            if !clicked_button && !clicked_popup {
                ui.ctx()
                    .memory_mut(|m| m.data.insert_temp::<bool>(popup_id, false));
            }
        }
    }

    chosen
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
        ui.heading(t!("tags.add"));
        ui.horizontal(|ui| {
            ui.label(t!("tags.name"));
            ui.add_sized(
                egui::vec2(180.0, 22.0),
                egui::TextEdit::singleline(&mut app.tag_manager_new_name),
            );
            if let Some(color) = color_picker(
                ui,
                egui::Id::new("tag_manager_new_color_picker"),
                app.tag_manager_new_color,
            ) {
                app.tag_manager_new_color = color;
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
                ui.add_sized(
                    egui::vec2(160.0, 22.0),
                    egui::TextEdit::singleline(edit_name),
                );
                if ui.button(t!("tags.save")).clicked() {
                    actions.push(TagManagerAction::Rename(tag.id, edit_name.clone()));
                }
                if let Some(color) = color_picker(
                    ui,
                    egui::Id::new(("tag_manager_color_picker", tag.id)),
                    tag.color,
                ) {
                    actions.push(TagManagerAction::Recolor(tag.id, color));
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
