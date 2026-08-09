mod geometry;
mod item_renderer;
mod scroll;

use eframe::egui::{self, Rect, Sense, Ui};

use self::geometry::{calculate_grouped_layout, calculate_layout, COLUMN_WIDTH, ROW_HEIGHT};
use self::item_renderer::{render_grouped_columns, render_visible_columns};
use super::list_view::{ListViewAction, ListViewContext, ListViewOperations};
use super::rectangle_selection::{
    ColumnListRectangleMetrics, GroupedProjectionIdentity, GroupedRectangleLayout,
    GroupedRectangleMetrics, GroupedRectangleSection, RectangleSelectionMetrics,
    RectangleSelectionView,
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

pub fn column_list_grouped_rows_for_counts(
    group_counts: &[usize],
    available_width: f32,
    available_height: f32,
) -> usize {
    calculate_grouped_layout(
        group_counts,
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
    ctx.visible_group_paths.clear();
    let grouped = ctx.group_projection.is_grouped() && !ctx.compact;
    let header_height = if grouped { COMPUTER_HEADER_HEIGHT } else { 0.0 };
    let layout_height = (available_rect.height() - header_height).max(0.0);
    let group_counts: Vec<usize> = ctx
        .group_projection
        .sections
        .iter()
        .map(|section| {
            if ctx.collapsed_groups.contains(&section.key) {
                1
            } else {
                section.item_indices.len().max(1)
            }
        })
        .collect();
    let layout = if grouped {
        calculate_grouped_layout(&group_counts, available_rect.width(), layout_height)
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
    let selected_layout_index = if grouped {
        ctx.selected_item
            .and_then(|selected| grouped_layout_index(selected, layout.rows_per_column, ctx))
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
    let background = ui.interact(
        viewport_rect,
        ui.id().with("column_list_bg"),
        Sense::click_and_drag(),
    );
    let rectangle_geometry_needed =
        ctx.rectangle_selection_state.is_some() || background.drag_started();
    let rectangle_metrics = if ctx.is_computer_view {
        None
    } else if grouped && rectangle_geometry_needed {
        Some(RectangleSelectionMetrics::Grouped(
            grouped_column_rectangle_metrics(
                ctx,
                layout.rows_per_column,
                layout.content_width,
                header_height + layout.rows_per_column as f32 * ROW_HEIGHT,
            ),
        ))
    } else if !grouped {
        Some(RectangleSelectionMetrics::ColumnList(
            ColumnListRectangleMetrics {
                count: ctx.items.len(),
                rows_per_column: layout.rows_per_column,
                column_width: COLUMN_WIDTH,
                row_height: ROW_HEIGHT,
                content_width: layout.content_width,
                content_height: layout.rows_per_column as f32 * ROW_HEIGHT,
            },
        ))
    } else {
        None
    };
    ctx.rectangle_selection_frame.begin(
        viewport_rect,
        current_scroll,
        0.0,
        max_scroll,
        0.0,
        rectangle_metrics,
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
    let mut secondary_clicked_empty_area = false;
    let mut toggled_group = None;
    if grouped {
        render_group_headers(
            ui,
            viewport_rect,
            current_scroll,
            layout.rows_per_column,
            ctx,
            &mut toggled_group,
        );
        let groups: Vec<(&[usize], bool)> = ctx
            .group_projection
            .sections
            .iter()
            .map(|section| {
                (
                    section.item_indices.as_ref(),
                    ctx.collapsed_groups.contains(&section.key),
                )
            })
            .collect();
        render_grouped_columns(
            ui,
            viewport_rect,
            viewport_rect.top() + COMPUTER_HEADER_HEIGHT,
            current_scroll,
            layout.rows_per_column,
            &groups,
            ctx,
            ops,
            &mut clicked_item,
            &mut double_clicked_item,
            &mut secondary_clicked_item,
            &mut secondary_clicked_empty_area,
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
            &mut secondary_clicked_empty_area,
        );
    }
    if grouped
        && ctx.rectangle_selection_frame.metrics.is_none()
        && ctx.rectangle_selection_frame.start_screen_pos.is_some()
    {
        ctx.rectangle_selection_frame.metrics = Some(RectangleSelectionMetrics::Grouped(
            grouped_column_rectangle_metrics(
                ctx,
                layout.rows_per_column,
                layout.content_width,
                header_height + layout.rows_per_column as f32 * ROW_HEIGHT,
            ),
        ));
    }

    if let Some(state) = ctx.rectangle_selection_state.filter(|state| {
        state.view == RectangleSelectionView::ColumnList && state.generation == ctx.generation
    }) {
        super::rectangle_selection::paint_overlay(ui, state, viewport_rect, current_scroll, 0.0);
    }

    if ctx.items.is_empty() {
        *ctx.visible_index_range = None;
    }

    if let Some(key) = toggled_group {
        Some(ListViewAction::ToggleGroup(key))
    } else if let Some(index) = double_clicked_item {
        Some(ListViewAction::DoubleClick(index))
    } else if let Some(index) = secondary_clicked_item {
        Some(ListViewAction::SecondaryClick(index))
    } else if let Some(index) = clicked_item {
        Some(ListViewAction::Click(index))
    } else if secondary_clicked_empty_area || background.secondary_clicked() {
        Some(ListViewAction::EmptyAreaSecondaryClick)
    } else if background.clicked() {
        Some(ListViewAction::EmptyAreaClick)
    } else {
        None
    }
}

fn grouped_column_rectangle_metrics(
    ctx: &ListViewContext,
    rows_per_column: usize,
    content_width: f32,
    content_height: f32,
) -> GroupedRectangleMetrics {
    let mut sections = Vec::new();
    let mut group_start_column = 0usize;
    for section in &ctx.group_projection.sections {
        if ctx.collapsed_groups.contains(&section.key) {
            group_start_column += 1;
            continue;
        }
        sections.push(GroupedRectangleSection {
            item_indices: section.item_indices.clone(),
            origin: egui::pos2(
                group_start_column as f32 * COLUMN_WIDTH,
                COMPUTER_HEADER_HEIGHT,
            ),
        });
        group_start_column += section.item_indices.len().div_ceil(rows_per_column).max(1);
    }
    let sections: std::sync::Arc<[GroupedRectangleSection]> = sections.into();
    GroupedRectangleMetrics {
        view: RectangleSelectionView::ColumnList,
        projection_identity: GroupedProjectionIdentity::new(sections.clone(), ctx.items.len()),
        sections,
        layout: GroupedRectangleLayout::ColumnList {
            rows_per_column,
            column_width: COLUMN_WIDTH,
            row_height: ROW_HEIGHT,
        },
        item_count: ctx.items.len(),
        content_width,
        content_height,
    }
}

fn grouped_layout_index(
    item_index: usize,
    rows_per_column: usize,
    ctx: &ListViewContext,
) -> Option<usize> {
    let mut start = 0usize;
    for section in &ctx.group_projection.sections {
        if ctx.collapsed_groups.contains(&section.key) {
            start += rows_per_column;
            continue;
        }
        if let Some(position) = section
            .item_indices
            .iter()
            .position(|index| *index == item_index)
        {
            return Some(start + position);
        }
        start += section.item_indices.len().div_ceil(rows_per_column).max(1) * rows_per_column;
    }
    None
}

fn render_group_headers(
    ui: &mut Ui,
    viewport_rect: Rect,
    scroll_x: f32,
    rows_per_column: usize,
    ctx: &ListViewContext,
    toggled_group: &mut Option<crate::application::grouping::GroupKey>,
) {
    let mut start_column = 0usize;
    for section in &ctx.group_projection.sections {
        let collapsed = ctx.collapsed_groups.contains(&section.key);
        let rect = Rect::from_min_size(
            egui::pos2(
                viewport_rect.left() + start_column as f32 * COLUMN_WIDTH - scroll_x,
                viewport_rect.top(),
            ),
            egui::vec2(COLUMN_WIDTH, COMPUTER_HEADER_HEIGHT),
        );
        if rect.intersects(viewport_rect)
            && crate::ui::views::group_header::render_group_header(
                ui,
                rect,
                &section.key,
                section.item_indices.len(),
                collapsed,
            )
            .clicked()
        {
            *toggled_group = Some(section.key.clone());
        }
        start_column += if collapsed {
            1
        } else {
            section.item_indices.len().div_ceil(rows_per_column).max(1)
        };
    }
}
