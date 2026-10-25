//! Horizontal layout for content-sized or manual columns. Vertical virtualization stays independent.

use super::ListViewContext;
use crate::domain::file_entry::SortMode;
use eframe::egui::{self, Rect, Sense, Ui};

const SCROLLBAR_HEIGHT: f32 = 14.0;

fn offset_to_reveal(current: f32, viewport: f32, start: f32, width: f32) -> f32 {
    if current > start {
        return start;
    }
    let end = start + width.min(viewport.max(0.0));
    if current + viewport < end {
        end - viewport
    } else {
        current
    }
}

pub(super) fn reveal_name_for_rename(ctx: &mut ListViewContext, available: f32) {
    if ctx.compact || !ctx.focus_rename || ctx.renaming_state.is_none() {
        return;
    }
    if let Some(cell) = super::columns::cells(ctx)
        .iter()
        .find(|cell| cell.id == crate::domain::list_columns::ColumnId::Name)
    {
        *ctx.mut_scroll_offset_x = offset_to_reveal(
            *ctx.mut_scroll_offset_x,
            (available - 8.0).max(0.0),
            cell.offset,
            cell.width,
        );
    }
}

pub(super) fn content_width(ctx: &ListViewContext, available_width: f32) -> f32 {
    if ctx.compact {
        return available_width;
    }
    required_width(
        super::columns::cells(ctx)
            .into_iter()
            .map(|cell| cell.width),
        available_width,
    )
}

fn required_width(widths: impl IntoIterator<Item = f32>, available_width: f32) -> f32 {
    (widths.into_iter().sum::<f32>() + 8.0).max(available_width)
}

pub(super) fn prepare(ui: &mut Ui, ctx: &mut ListViewContext) {
    let rect = ui.available_rect_before_wrap();
    let width = content_width(ctx, rect.width());
    prepare_scrollbar(
        ui,
        rect,
        width,
        ctx.mut_scroll_offset_x,
        ctx.global_search_active,
        ctx.scroll_interpolation_enabled,
    );
}

fn prepare_scrollbar(
    ui: &mut Ui,
    rect: Rect,
    width: f32,
    scroll_x: &mut f32,
    global_search_active: bool,
    interpolation_enabled: bool,
) {
    let max_scroll = (width - rect.width()).max(0.0);
    *scroll_x = scroll_x.clamp(0.0, max_scroll);
    if max_scroll <= 0.0 {
        return;
    }
    let pointer_over = ui.ctx().pointer_hover_pos().is_some_and(|pos| {
        rect.contains(pos)
            && ui
                .ctx()
                .layer_id_at(pos)
                .is_none_or(|layer| layer.order == egui::Order::Background)
    });
    if pointer_over && !global_search_active {
        let delta = ui.input(|input| {
            if interpolation_enabled {
                input.smooth_scroll_delta.x
            } else {
                crate::ui::views::common::raw_wheel_delta(input, rect.height()).x
            }
        });
        *scroll_x = (*scroll_x - delta * 5.0).clamp(0.0, max_scroll);
    }
    let track = Rect::from_min_max(
        egui::pos2(rect.left(), rect.bottom() - SCROLLBAR_HEIGHT),
        rect.max,
    );
    let response = ui.interact(
        track,
        ui.id().with("list_horizontal_scrollbar"),
        Sense::click_and_drag(),
    );
    let handle_width = (track.width() * track.width() / width)
        .max(30.0)
        .min(track.width());
    let travel = (track.width() - handle_width).max(1.0);
    let previous_scroll = *scroll_x;
    if response.dragged() {
        *scroll_x =
            (*scroll_x + response.drag_delta().x * max_scroll / travel).clamp(0.0, max_scroll);
    } else if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let handle_left = track.left() + *scroll_x / max_scroll * travel;
            // Clicking the thumb must not recenter it under the pointer.
            if pos.x < handle_left || pos.x > handle_left + handle_width {
                *scroll_x = ((pos.x - track.left() - handle_width * 0.5) / travel * max_scroll)
                    .clamp(0.0, max_scroll);
            }
        }
    }
    if *scroll_x != previous_scroll {
        ui.ctx().request_repaint();
    }
    let left = track.left() + *scroll_x / max_scroll * travel;
    let handle = Rect::from_min_size(
        egui::pos2(left, track.top() + 4.0),
        egui::vec2(handle_width, 6.0),
    );
    ui.painter().rect_filled(
        handle,
        3.0,
        ui.visuals()
            .widgets
            .inactive
            .fg_stroke
            .color
            .gamma_multiply(0.65),
    );
}

pub(super) fn render_header(
    ui: &mut Ui,
    ctx: &mut ListViewContext,
    available_w: f32,
) -> Option<SortMode> {
    if content_width(ctx, available_w) <= available_w {
        return super::header::render_list_header(ui, ctx, available_w);
    }
    let rect = Rect::from_min_size(ui.cursor().min, egui::vec2(available_w, 22.0));
    let content_rect = Rect::from_min_size(
        rect.min - egui::vec2(*ctx.mut_scroll_offset_x, 0.0),
        egui::vec2(content_width(ctx, available_w), 22.0),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(content_rect));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let action = super::header::render_list_header(&mut child, ctx, available_w);
    ui.allocate_exact_size(rect.size(), Sense::hover());
    action
}

pub(super) fn viewport(ui: &Ui, ctx: &ListViewContext) -> Rect {
    let mut rect = ui.available_rect_before_wrap();
    if content_width(ctx, rect.width()) > rect.width() {
        rect.max.y = (rect.bottom() - SCROLLBAR_HEIGHT).max(rect.top());
    }
    rect
}

#[cfg(test)]
#[path = "horizontal_tests.rs"]
mod tests;
