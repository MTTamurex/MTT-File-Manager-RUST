mod geometry;
mod item_renderer;
mod scroll;

use eframe::egui::{self, Rect, Sense, Ui};

use self::geometry::{calculate_layout, COLUMN_WIDTH, ROW_HEIGHT};
use self::item_renderer::render_visible_columns;
use super::list_view::{ListViewAction, ListViewContext, ListViewOperations};
use super::rectangle_selection::{
    ColumnListRectangleMetrics, RectangleSelectionMetrics, RectangleSelectionView,
};
pub fn column_list_rows(item_count: usize, available_width: f32, available_height: f32) -> usize {
    calculate_layout(item_count, available_width, available_height).rows_per_column
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
    let layout = calculate_layout(
        ctx.items.len(),
        available_rect.width(),
        available_rect.height(),
    );
    let viewport_rect = Rect::from_min_max(
        available_rect.min,
        egui::pos2(
            available_rect.right(),
            available_rect.top() + layout.viewport_height,
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
    scroll::ensure_selected_visible(
        ctx.selected_item,
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
