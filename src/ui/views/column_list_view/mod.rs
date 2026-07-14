mod geometry;
mod item_renderer;
mod scroll;

use eframe::egui::{self, Color32, FontId, Rect, Sense, Ui};
use rust_i18n::t;

use self::geometry::{calculate_grouped_layout, calculate_layout, COLUMN_WIDTH, ROW_HEIGHT};
use self::item_renderer::{render_grouped_columns, render_visible_columns};
use super::list_view::{ListViewAction, ListViewContext, ListViewOperations};
use super::rectangle_selection::{
    ColumnListRectangleMetrics, RectangleSelectionMetrics, RectangleSelectionView,
};
const COMPUTER_HEADER_HEIGHT: f32 = 28.0;

pub fn column_list_rows(item_count: usize, available_width: f32, available_height: f32) -> usize {
    calculate_layout(item_count, available_width, available_height).rows_per_column
}

pub fn column_list_grouped_rows(
    local_count: usize,
    network_count: usize,
    available_width: f32,
    available_height: f32,
) -> usize {
    calculate_grouped_layout(
        &[local_count, network_count],
        available_width,
        (available_height - COMPUTER_HEADER_HEIGHT).max(0.0),
    )
    .rows_per_column
}

pub fn column_list_visible_columns(available_width: f32) -> usize {
    (available_width / COLUMN_WIDTH).floor().max(1.0) as usize
}

pub fn render_column_list_view(
    ui: &mut Ui,
    ctx: &mut ListViewContext,
    ops: &mut dyn ListViewOperations,
) -> Option<ListViewAction> {
    let available_rect = ui.available_rect_before_wrap();
    let mut local_indices = Vec::new();
    let mut network_indices = Vec::new();
    if ctx.is_computer_view {
        for (index, item) in ctx.items.iter().enumerate() {
            let is_remote = item.drive_info.as_ref().is_some_and(|drive| {
                drive.drive_type == crate::infrastructure::windows::DriveType::Remote
            });
            if is_remote {
                network_indices.push(index);
            } else {
                local_indices.push(index);
            }
        }
    }
    let header_height = if ctx.is_computer_view {
        COMPUTER_HEADER_HEIGHT
    } else {
        0.0
    };
    let layout_height = (available_rect.height() - header_height).max(0.0);
    let layout = if ctx.is_computer_view {
        calculate_grouped_layout(
            &[local_indices.len(), network_indices.len()],
            available_rect.width(),
            layout_height,
        )
    } else {
        calculate_layout(ctx.items.len(), available_rect.width(), layout_height)
    };
    let viewport_rect = Rect::from_min_max(
        available_rect.min,
        egui::pos2(
            available_rect.right(),
            available_rect.top() + header_height + layout.viewport_height,
        ),
    );
    let max_scroll = (layout.content_width - viewport_rect.width()).max(0.0);

    scroll::apply_input(
        ui,
        viewport_rect,
        ctx.mut_scroll_offset_x,
        max_scroll,
        ctx.global_search_active,
    );
    let selected_layout_index = if ctx.is_computer_view {
        ctx.selected_item.and_then(|selected| {
            grouped_layout_index(
                selected,
                layout.rows_per_column,
                &local_indices,
                &network_indices,
            )
        })
    } else {
        ctx.selected_item
    };
    scroll::ensure_selected_visible(
        selected_layout_index,
        ctx.scroll_to_selected,
        layout.rows_per_column,
        viewport_rect.width(),
        max_scroll,
        ctx.mut_scroll_offset_x,
    );
    if layout.has_horizontal_scrollbar && max_scroll > 0.0 {
        scroll::render_scrollbar(
            ui,
            available_rect,
            layout.content_width,
            max_scroll,
            ctx.mut_scroll_offset_x,
        );
    }

    let current_scroll = *ctx.mut_scroll_offset_x;
    let rectangle_metrics = (!ctx.is_computer_view).then_some(
        RectangleSelectionMetrics::ColumnList(ColumnListRectangleMetrics {
            count: ctx.items.len(),
            rows_per_column: layout.rows_per_column,
            column_width: COLUMN_WIDTH,
            row_height: ROW_HEIGHT,
            content_width: layout.content_width,
            content_height: layout.rows_per_column as f32 * ROW_HEIGHT,
        }),
    );
    ctx.rectangle_selection_frame.begin(
        viewport_rect,
        current_scroll,
        0.0,
        max_scroll,
        0.0,
        rectangle_metrics,
    );

    let background = ui.interact(
        viewport_rect,
        ui.id().with("column_list_bg"),
        Sense::click_and_drag(),
    );
    if !ctx.is_computer_view && ctx.rectangle_selection_state.is_none() && background.drag_started()
    {
        if let Some(origin) = ui.input(|input| input.pointer.press_origin()) {
            ctx.rectangle_selection_frame.request_start(origin);
        }
    }

    let mut clicked_item = None;
    let mut double_clicked_item = None;
    let mut secondary_clicked_item = None;
    if ctx.is_computer_view {
        render_computer_headers(
            ui,
            viewport_rect,
            current_scroll,
            layout.rows_per_column,
            &local_indices,
            &network_indices,
        );
        render_grouped_columns(
            ui,
            viewport_rect,
            viewport_rect.top() + COMPUTER_HEADER_HEIGHT,
            current_scroll,
            layout.rows_per_column,
            &[&local_indices, &network_indices],
            ctx,
            ops,
            &mut clicked_item,
            &mut double_clicked_item,
            &mut secondary_clicked_item,
        );
    } else {
        render_visible_columns(
            ui,
            viewport_rect,
            current_scroll,
            layout.rows_per_column,
            ctx,
            ops,
            &mut clicked_item,
            &mut double_clicked_item,
            &mut secondary_clicked_item,
        );
    }

    if let Some(state) = ctx.rectangle_selection_state.filter(|state| {
        state.view == RectangleSelectionView::ColumnList && state.generation == ctx.generation
    }) {
        super::rectangle_selection::paint_overlay(ui, state, viewport_rect, current_scroll, 0.0);
    }

    if ctx.items.is_empty() {
        *ctx.visible_index_range = None;
    }

    if let Some(index) = double_clicked_item {
        Some(ListViewAction::DoubleClick(index))
    } else if let Some(index) = secondary_clicked_item {
        Some(ListViewAction::SecondaryClick(index))
    } else if let Some(index) = clicked_item {
        Some(ListViewAction::Click(index))
    } else if background.secondary_clicked() {
        Some(ListViewAction::EmptyAreaSecondaryClick)
    } else if background.clicked() {
        Some(ListViewAction::EmptyAreaClick)
    } else {
        None
    }
}

fn grouped_layout_index(
    item_index: usize,
    rows_per_column: usize,
    local_indices: &[usize],
    network_indices: &[usize],
) -> Option<usize> {
    if let Some(position) = local_indices.iter().position(|index| *index == item_index) {
        return Some(position);
    }
    let network_position = network_indices
        .iter()
        .position(|index| *index == item_index)?;
    let network_start = local_indices.len().div_ceil(rows_per_column) * rows_per_column;
    Some(network_start + network_position)
}

fn render_computer_headers(
    ui: &mut Ui,
    viewport_rect: Rect,
    scroll_x: f32,
    rows_per_column: usize,
    local_indices: &[usize],
    network_indices: &[usize],
) {
    let mut start_column = 0usize;
    for (title, indices) in [
        (t!("sidebar.local_disks"), local_indices),
        (t!("sidebar.network_drives"), network_indices),
    ] {
        if indices.is_empty() {
            continue;
        }
        let rect = Rect::from_min_size(
            egui::pos2(
                viewport_rect.left() + start_column as f32 * COLUMN_WIDTH - scroll_x,
                viewport_rect.top(),
            ),
            egui::vec2(COLUMN_WIDTH, COMPUTER_HEADER_HEIGHT),
        );
        if rect.intersects(viewport_rect) {
            ui.painter().text(
                egui::pos2(rect.left() + 8.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                title,
                FontId::proportional(13.0),
                Color32::from_gray(120),
            );
        }
        start_column += indices.len().div_ceil(rows_per_column);
    }
}
