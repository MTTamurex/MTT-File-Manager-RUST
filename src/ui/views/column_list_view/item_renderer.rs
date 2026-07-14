use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, Ui};

use super::geometry::{COLUMN_WIDTH, ROW_HEIGHT};
use crate::domain::file_entry::FileEntry;
use crate::ui::views::list_view::item_renderer::prepare_list_item_resources;
use crate::ui::views::list_view::item_renderer_details::{render_item_icon, render_item_tooltip};
use crate::ui::views::list_view::{truncate_text_for_column, ListViewContext, ListViewOperations};

#[allow(clippy::too_many_arguments)]
pub(super) fn render_visible_columns(
    ui: &mut Ui,
    viewport_rect: Rect,
    scroll_x: f32,
    rows_per_column: usize,
    ctx: &mut ListViewContext,
    ops: &mut dyn ListViewOperations,
    clicked_item: &mut Option<usize>,
    double_clicked_item: &mut Option<usize>,
    secondary_clicked_item: &mut Option<usize>,
) {
    let column_count = ctx.items.len().div_ceil(rows_per_column);
    let first_column = ((scroll_x / COLUMN_WIDTH).floor() as usize).saturating_sub(1);
    let last_column =
        (((scroll_x + viewport_rect.width()) / COLUMN_WIDTH).ceil() as usize + 1).min(column_count);
    let first_index = first_column * rows_per_column;
    let last_index = (last_column * rows_per_column).min(ctx.items.len());
    *ctx.visible_index_range = (first_index < last_index).then_some((first_index, last_index - 1));

    let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(viewport_rect));
    child_ui.set_clip_rect(viewport_rect.intersect(ui.clip_rect()));

    for column in first_column..last_column {
        for row in 0..rows_per_column {
            let index = column * rows_per_column + row;
            if index >= ctx.items.len() {
                break;
            }
            let rect = Rect::from_min_size(
                egui::pos2(
                    viewport_rect.left() + column as f32 * COLUMN_WIDTH - scroll_x,
                    viewport_rect.top() + row as f32 * ROW_HEIGHT,
                ),
                egui::vec2(COLUMN_WIDTH, ROW_HEIGHT),
            );
            render_column_item(
                &mut child_ui,
                index,
                rect,
                ctx,
                ops,
                clicked_item,
                double_clicked_item,
                secondary_clicked_item,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_grouped_columns(
    ui: &mut Ui,
    viewport_rect: Rect,
    item_top: f32,
    scroll_x: f32,
    rows_per_column: usize,
    groups: &[&[usize]],
    ctx: &mut ListViewContext,
    ops: &mut dyn ListViewOperations,
    clicked_item: &mut Option<usize>,
    double_clicked_item: &mut Option<usize>,
    secondary_clicked_item: &mut Option<usize>,
) {
    *ctx.visible_index_range = (!ctx.items.is_empty()).then_some((0, ctx.items.len() - 1));
    let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(viewport_rect));
    child_ui.set_clip_rect(viewport_rect.intersect(ui.clip_rect()));
    let mut group_start_column = 0usize;

    for indices in groups {
        for (group_index, &item_index) in indices.iter().enumerate() {
            if item_index >= ctx.items.len() {
                continue;
            }
            let column = group_start_column + group_index / rows_per_column;
            let row = group_index % rows_per_column;
            let rect = Rect::from_min_size(
                egui::pos2(
                    viewport_rect.left() + column as f32 * COLUMN_WIDTH - scroll_x,
                    item_top + row as f32 * ROW_HEIGHT,
                ),
                egui::vec2(COLUMN_WIDTH, ROW_HEIGHT),
            );
            if child_ui.is_rect_visible(rect) {
                render_column_item(
                    &mut child_ui,
                    item_index,
                    rect,
                    ctx,
                    ops,
                    clicked_item,
                    double_clicked_item,
                    secondary_clicked_item,
                );
            }
        }
        group_start_column += indices.len().div_ceil(rows_per_column);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_column_item(
    ui: &mut Ui,
    index: usize,
    rect: Rect,
    ctx: &mut ListViewContext,
    ops: &mut dyn ListViewOperations,
    clicked_item: &mut Option<usize>,
    double_clicked_item: &mut Option<usize>,
    secondary_clicked_item: &mut Option<usize>,
) {
    let item = &ctx.items[index];
    prepare_list_item_resources(ui, index, item, ctx, ops);
    let item = &ctx.items[index];

    ui.push_id(("column_item", index), |ui| {
        let response = ui.interact(rect, ui.id().with(index), Sense::click_and_drag());
        if response.clicked() {
            *clicked_item = Some(index);
        }
        if response.double_clicked() {
            *double_clicked_item = Some(index);
        }
        if response.secondary_clicked() {
            *secondary_clicked_item = Some(index);
        }

        let (press_origin, pointer_pos) =
            ui.input(|input| (input.pointer.press_origin(), input.pointer.hover_pos()));
        let drag_candidate = crate::ui::views::common::should_start_item_drag(
            response.drag_started(),
            response.dragged(),
            response.is_pointer_button_down_on(),
            press_origin,
            pointer_pos,
        );
        if drag_candidate && ctx.rectangle_selection_state.is_none() {
            if ctx.is_computer_view {
                *ctx.drag_started_item = Some(index);
            } else if let Some(origin) = ui.input(|input| input.pointer.press_origin()) {
                if column_item_content_contains_pointer(ui, item, rect, ctx, origin) {
                    *ctx.drag_started_item = Some(index);
                } else {
                    ctx.rectangle_selection_frame.request_start(origin);
                }
            }
        }
        if response.contains_pointer() && item.is_dir {
            *ctx.drag_hovered_item = Some(index);
        }

        let is_selected = ctx
            .rectangle_selection_state
            .map(|state| state.preview_contains(index))
            .unwrap_or_else(|| ctx.multi_selection.contains(&item.path));
        let allow_hover = matches!(ctx.last_input, crate::app::state::LastInput::Mouse);
        let is_hovered = allow_hover && response.hovered() && !is_selected;
        let is_focused = ctx.selected_item == Some(index);
        let accent = crate::ui::theme::COLOR_ACCENT;
        let visual_rect = rect.shrink2(egui::vec2(2.0, 0.0));

        if is_selected {
            ui.painter().rect_stroke(
                visual_rect,
                4.0,
                egui::Stroke::new(2.0, accent),
                egui::StrokeKind::Inside,
            );
        } else if is_hovered || is_focused {
            ui.painter().rect_stroke(
                visual_rect,
                4.0,
                egui::Stroke::new(1.0, accent.gamma_multiply(0.35)),
                egui::StrokeKind::Inside,
            );
        }

        if ctx.is_item_dragging && item.is_dir && response.contains_pointer() {
            ui.painter().rect_stroke(
                visual_rect.shrink(1.0),
                4.0,
                egui::Stroke::new(2.0, accent),
                egui::StrokeKind::Inside,
            );
        }

        if !ctx.is_item_dragging && ctx.rectangle_selection_state.is_none() {
            render_item_tooltip(ui, &response, item, ctx, ctx.is_recycle_bin_view);
        }

        let opacity = if item.is_hidden { 0.5 } else { 1.0 };
        render_item_icon(
            ui,
            item,
            ctx,
            ops,
            rect,
            Color32::WHITE.gamma_multiply(opacity),
        );
        render_column_item_name(ui, index, item, rect, is_selected, opacity, ctx, ops);
    });
}

fn column_item_content_contains_pointer(
    ui: &Ui,
    item: &FileEntry,
    rect: Rect,
    ctx: &ListViewContext,
    point: Pos2,
) -> bool {
    let icon_rect = Rect::from_min_size(rect.min + egui::vec2(4.0, 4.0), egui::vec2(16.0, 16.0));
    if icon_rect.expand(2.0).contains(point) {
        return true;
    }

    let has_tag = crate::domain::file_tag::tag_ids_for_path(ctx.tag_assignments, &item.path)
        .is_some_and(|ids| ids.iter().any(|id| ctx.tag_definitions.contains_key(id)));
    let tag_offset = if has_tag { 12.0 } else { 0.0 };
    let max_width = COLUMN_WIDTH - 32.0 - tag_offset;
    let font = FontId::proportional(12.0);
    let name = crate::ui::components::item_slot::display_name_for_item(item);
    let display_name = truncate_text_for_column(name.as_ref(), max_width, &font, ui);
    let text_width = ui.fonts(|fonts| {
        fonts
            .layout_no_wrap(display_name, font, Color32::WHITE)
            .rect
            .width()
    });
    Rect::from_min_size(
        rect.min + egui::vec2(24.0 + tag_offset, 3.0),
        egui::vec2(text_width.min(max_width), ROW_HEIGHT - 6.0),
    )
    .expand(3.0)
    .contains(point)
}

#[allow(clippy::too_many_arguments)]
fn render_column_item_name(
    ui: &mut Ui,
    index: usize,
    item: &FileEntry,
    rect: Rect,
    is_selected: bool,
    opacity: f32,
    ctx: &mut ListViewContext,
    ops: &mut dyn ListViewOperations,
) {
    let tag_color = crate::domain::file_tag::tag_ids_for_path(ctx.tag_assignments, &item.path)
        .and_then(|ids| ids.iter().find_map(|id| ctx.tag_definitions.get(id)))
        .map(|tag| tag.color.to_color32());
    let tag_offset = if tag_color.is_some() { 12.0 } else { 0.0 };
    if let Some(color) = tag_color {
        ui.painter().circle_filled(
            Pos2::new(rect.left() + 27.0, rect.center().y),
            3.2,
            color.gamma_multiply(opacity),
        );
    }

    if ctx
        .renaming_state
        .as_ref()
        .is_some_and(|(i, _)| *i == index)
    {
        let mut text = ctx
            .renaming_state
            .as_ref()
            .map(|(_, text)| text.clone())
            .unwrap_or_default();
        let edit_rect = Rect::from_min_size(
            rect.min + egui::vec2(24.0 + tag_offset, 2.0),
            egui::vec2(COLUMN_WIDTH - 32.0 - tag_offset, ROW_HEIGHT - 4.0),
        );
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(edit_rect), |ui| {
            let response =
                ui.add(egui::TextEdit::singleline(&mut text).id_source("rename_input_column_list"));
            if ctx.focus_rename {
                response.request_focus();
            }
            if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                ops.rename_with_shell(index);
            }
        });
        if let Some((_, rename_text)) = ctx.renaming_state.as_mut() {
            *rename_text = text;
        }
        return;
    }

    let font = FontId::proportional(12.0);
    let name = crate::ui::components::item_slot::display_name_for_item(item);
    let display_name =
        truncate_text_for_column(name.as_ref(), COLUMN_WIDTH - 32.0 - tag_offset, &font, ui);
    let color = if is_selected {
        crate::ui::theme::selection_text_color(ui.visuals().dark_mode)
    } else {
        crate::ui::theme::text_color(ui.visuals().dark_mode)
    }
    .gamma_multiply(opacity);
    ui.painter().text(
        rect.min + egui::vec2(24.0 + tag_offset, 5.0),
        egui::Align2::LEFT_TOP,
        display_name,
        font,
        color,
    );
}
