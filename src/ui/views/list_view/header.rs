//! Sortable, resizable, movable column headers and their context menu.
use super::{
    autofit, column_picker, columns, fit_panel, truncate_text_for_column, ListViewContext,
};
use crate::app::explorer_preferences::ListColumnSizeMode;
use crate::domain::file_entry::SortMode;
use crate::domain::list_columns::{ColumnId, ColumnLayout};
use crate::ui::tab_drag_preview::TabDragPreview;
use eframe::egui::{self, Color32, FontId, Sense, Ui};

struct HeaderResponse {
    clicked: bool,
    dragging: bool,
    stopped: bool,
    rect: egui::Rect,
    response: egui::Response,
    handle_response: egui::Response,
}

#[allow(clippy::too_many_arguments)]
fn draw_header(
    ui: &mut Ui,
    text: &str,
    width: &mut f32,
    mode: Option<SortMode>,
    min_width: f32,
    other_widths: f32,
    sort_mode: SortMode,
    sort_descending: bool,
    available_for_columns: f32,
) -> HeaderResponse {
    let rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(*width, 22.0));
    let body = egui::Rect::from_min_max(
        rect.min + egui::vec2(4.0, 0.0),
        rect.max - egui::vec2(4.0, 0.0),
    );
    let response = ui.interact(body, ui.id().with("header"), Sense::click_and_drag());
    let hovered = response.hovered()
        && ui.input(|input| {
            input
                .pointer
                .hover_pos()
                .is_some_and(|pos| body.contains(pos))
        });
    if ui.is_rect_visible(rect) {
        let lifted = response.dragged_by(egui::PointerButton::Primary)
            || response.drag_stopped_by(egui::PointerButton::Primary);
        if hovered || lifted {
            ui.painter()
                .rect_filled(rect, 2.0, crate::ui::theme::header_active_bg(ui.ctx()));
        }
        let color = if hovered || lifted {
            crate::ui::theme::text_color(ui.ctx())
        } else {
            crate::ui::theme::secondary_text_color(ui.ctx())
        };
        let font = FontId::proportional(12.0);
        let text = truncate_text_for_column(text, *width - 30.0, &font, ui);
        ui.painter().text(
            rect.min + egui::vec2(8.0, 4.0),
            egui::Align2::LEFT_TOP,
            text,
            font,
            color,
        );
        if mode == Some(sort_mode) {
            ui.painter().text(
                rect.max - egui::vec2(15.0, 8.0),
                egui::Align2::CENTER_CENTER,
                if sort_descending { "▼" } else { "▲" },
                FontId::proportional(10.0),
                color,
            );
        }
    }
    let handle_rect = egui::Rect::from_min_size(
        egui::pos2(rect.max.x - 4.0, rect.min.y),
        egui::vec2(8.0, 22.0),
    );
    let handle = ui.interact(handle_rect, ui.id().with("resize"), Sense::click_and_drag());
    let resizing = handle.dragged_by(egui::PointerButton::Primary);
    if handle.hovered() || resizing {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        ui.painter().rect_filled(
            handle_rect.shrink2(egui::vec2(2.0, 4.0)),
            0.0,
            Color32::from_rgb(100, 150, 200),
        );
    }
    if resizing {
        *width = (*width + handle.drag_delta().x).clamp(
            min_width,
            (available_for_columns - other_widths).max(min_width),
        );
    }
    ui.allocate_exact_size(egui::vec2(*width, 22.0), Sense::hover());
    HeaderResponse {
        clicked: response.clicked(),
        dragging: response.dragged_by(egui::PointerButton::Primary),
        stopped: response.drag_stopped_by(egui::PointerButton::Primary),
        rect,
        response,
        handle_response: handle,
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn draw_header_resizable(
    ui: &mut Ui,
    text: &str,
    width: &mut f32,
    mode: SortMode,
    min_width: f32,
    other_widths: f32,
    sort_mode: SortMode,
    sort_descending: bool,
    available_for_columns: f32,
) -> bool {
    draw_header(
        ui,
        text,
        width,
        Some(mode),
        min_width,
        other_widths,
        sort_mode,
        sort_descending,
        available_for_columns,
    )
    .clicked
}

struct HeaderSpec {
    cell: columns::ColumnCell,
    label: String,
    mode: Option<SortMode>,
}

struct HeaderRow {
    sort: Option<SortMode>,
    resized: Vec<(ColumnId, f32)>,
    responses: Vec<egui::Response>,
}

fn render_row(
    ui: &mut Ui,
    layout: &mut ColumnLayout,
    headers: Vec<HeaderSpec>,
    available_w: f32,
    sort_mode: SortMode,
    descending: bool,
) -> HeaderRow {
    let drag_id = ui.id().with("column_header_drag");
    let cancelled = ui.input(|input| {
        input.key_pressed(egui::Key::Escape)
            || !input.focused
            || (!input.pointer.primary_down() && !input.pointer.primary_released())
    });
    if cancelled {
        if ui.data(|data| data.get_temp::<ColumnId>(drag_id)).is_some() {
            ui.ctx().stop_dragging();
        }
        ui.data_mut(|data| data.remove::<ColumnId>(drag_id));
    }
    let total: f32 = headers.iter().map(|header| header.cell.width).sum();
    let start = ui.cursor().min;
    let bar = egui::Rect::from_min_size(start, egui::vec2(available_w.max(total), 22.0));
    // Register before children so the body and resize widgets keep their primary interactions.
    let bar_response = ui.interact(bar, ui.id().with("column_header_bar"), Sense::click());
    let mut sort = None;
    let mut rects = Vec::new();
    let mut stopped = false;
    let mut responses = vec![bar_response];
    let mut resized = Vec::new();
    let mut preview = TabDragPreview::default();
    let mut owner = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for HeaderSpec { cell, label, mode } in headers {
            let paint_start = TabDragPreview::begin(ui);
            let mut width = cell.width;
            let response = ui
                .push_id(cell.id, |ui| {
                    draw_header(
                        ui,
                        &label,
                        &mut width,
                        mode,
                        fit_panel::min_width(cell.id),
                        total - cell.width,
                        sort_mode,
                        descending,
                        f32::INFINITY,
                    )
                })
                .inner;
            if width != cell.width {
                resized.push((cell.id, width));
            }
            if response.clicked {
                sort = mode;
            }
            if response.dragging && !cancelled {
                ui.data_mut(|data| data.insert_temp(drag_id, cell.id));
            }
            if response.dragging || response.stopped {
                owner = Some(cell.id);
            }
            stopped |= response.stopped;
            preview.record(ui, response.rect, paint_start);
            responses.push(response.response);
            responses.push(response.handle_response);
            rects.push((cell.id, response.rect));
        }
    });
    let pointer = ui.input(|input| input.pointer.interact_pos());
    let strip = bar.intersect(ui.clip_rect());
    let source = ui
        .data(|data| data.get_temp::<ColumnId>(drag_id))
        .filter(|source| Some(*source) == owner);
    let gap = pointer.filter(|pos| strip.contains(*pos)).map(|pos| {
        rects
            .iter()
            .position(|(_, rect)| pos.x < rect.center().x)
            .unwrap_or(rects.len())
    });
    let source_index = source.and_then(|source| rects.iter().position(|(id, _)| *id == source));
    preview.paint_at_gap(ui, drag_id.with("preview"), source_index, strip, gap);
    if let Some(source) = source {
        if let Some(gap) = gap.filter(|_| stopped) {
            layout.move_before(source, rects.get(gap).map(|(id, _)| *id));
        }
        if stopped || !ui.input(|input| input.pointer.primary_down()) {
            ui.data_mut(|data| data.remove::<ColumnId>(drag_id));
        }
        sort = None;
    } else {
        ui.data_mut(|data| data.remove::<ColumnId>(drag_id));
    }
    HeaderRow {
        sort,
        resized,
        responses,
    }
}

pub(super) fn render_list_header(
    ui: &mut Ui,
    ctx: &mut ListViewContext,
    available_w: f32,
) -> Option<SortMode> {
    let scope = columns::scope(ctx);
    let headers = columns::cells(ctx)
        .into_iter()
        .map(|cell| {
            let mode = columns::sort_mode(cell.id);
            let mut label = columns::label(ctx, cell.id);
            if mode == Some(ctx.sort_mode) && ctx.column_sort_pending {
                label.push_str(" …");
            }
            HeaderSpec { label, mode, cell }
        })
        .collect();
    let row = render_row(
        ui,
        ctx.columns.preferences.layout_mut(scope),
        headers,
        available_w,
        ctx.sort_mode,
        ctx.sort_descending,
    );
    for (id, width) in row.resized {
        if ctx.column_size_mode == ListColumnSizeMode::FitToPanel {
            autofit::on_header_resize(ui, ctx, id, width);
        } else {
            columns::set_width(ctx, id, width);
        }
    }
    let owner = ctx.rename_click_scope;
    for response in row.responses {
        let response = if ctx.column_sort_pending {
            response.on_hover_text(rust_i18n::t!("column_headers.sorting"))
        } else {
            response
        };
        response.context_menu(|ui| column_picker::menu(ui, ctx, owner));
    }
    column_picker::dialog(ui, ctx);
    row.sort
}

#[cfg(test)]
#[path = "header_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "header_layout_tests.rs"]
mod layout_tests;

#[cfg(test)]
#[path = "header_drag_preview_tests.rs"]
mod drag_preview_tests;
