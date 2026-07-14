use eframe::egui::{self, Rect, Sense, Ui};

use super::geometry::{COLUMN_WIDTH, SCROLLBAR_HEIGHT};

pub(super) fn apply_input(
    ui: &Ui,
    viewport_rect: Rect,
    scroll_x: &mut f32,
    max_scroll: f32,
    global_search_active: bool,
) {
    let pointer_over = ui.ctx().pointer_hover_pos().is_some_and(|pos| {
        viewport_rect.contains(pos)
            && ui
                .ctx()
                .layer_id_at(pos)
                .is_none_or(|layer| layer.order == egui::Order::Background)
    });
    if pointer_over && !global_search_active {
        let delta = ui.input(|input| input.smooth_scroll_delta);
        let horizontal_delta = if delta.x.abs() > 0.01 {
            delta.x
        } else {
            delta.y
        };
        *scroll_x -= horizontal_delta * 5.0;
    }
    *scroll_x = scroll_x.clamp(0.0, max_scroll);
}

pub(super) fn ensure_selected_visible(
    selected_item: Option<usize>,
    requested: bool,
    rows_per_column: usize,
    viewport_width: f32,
    max_scroll: f32,
    scroll_x: &mut f32,
) {
    if !requested {
        return;
    }
    let Some(index) = selected_item else {
        return;
    };
    let item_left = (index / rows_per_column) as f32 * COLUMN_WIDTH;
    let item_right = item_left + COLUMN_WIDTH;
    if item_left < *scroll_x {
        *scroll_x = item_left;
    } else if item_right > *scroll_x + viewport_width {
        *scroll_x = (item_right - viewport_width).clamp(0.0, max_scroll);
    }
}

pub(super) fn render_scrollbar(
    ui: &mut Ui,
    available_rect: Rect,
    content_width: f32,
    max_scroll: f32,
    scroll_x: &mut f32,
) {
    let track = Rect::from_min_size(
        egui::pos2(
            available_rect.left(),
            available_rect.bottom() - SCROLLBAR_HEIGHT,
        ),
        egui::vec2(available_rect.width(), SCROLLBAR_HEIGHT),
    );
    let response = ui.interact(
        track,
        ui.id().with("column_list_scrollbar"),
        Sense::click_and_drag(),
    );
    let handle_width = (track.width() / content_width * track.width())
        .max(30.0)
        .min(track.width());
    let travel = (track.width() - handle_width).max(1.0);

    let previous_scroll = *scroll_x;
    if response.dragged() || response.clicked() {
        if let Some(pos) = ui.input(|input| input.pointer.interact_pos()) {
            *scroll_x = ((pos.x - track.left() - handle_width * 0.5) / travel * max_scroll)
                .clamp(0.0, max_scroll);
        }
    }
    if (*scroll_x - previous_scroll).abs() > 0.1 {
        ui.ctx().request_repaint();
    }

    let handle_left = track.left() + (*scroll_x / max_scroll) * travel;
    let handle = Rect::from_min_size(
        egui::pos2(handle_left, track.top() + 4.0),
        egui::vec2(handle_width, 6.0),
    );

    let color = ui
        .visuals()
        .widgets
        .inactive
        .fg_stroke
        .color
        .gamma_multiply(0.65);
    ui.painter().rect_filled(handle, 3.0, color);
}
