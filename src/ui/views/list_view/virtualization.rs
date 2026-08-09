//! Manual virtualization, scroll handling, scrollbar, and prefetch logic

use eframe::egui::{self, Color32, Rect, Sense, Ui};

use super::item_renderer::render_list_item;
use super::scroll;
use super::{ColumnWidths, ListViewContext, ListViewOperations};
use crate::application::grouping::GroupKey;
use crate::ui::views::group_header::{render_group_header, GROUP_GAP, GROUP_HEADER_HEIGHT};
use crate::ui::views::rectangle_selection::{
    GroupedProjectionIdentity, GroupedRectangleLayout, GroupedRectangleMetrics,
    GroupedRectangleSection, ListRectangleMetrics, RectangleSelectionMetrics,
    RectangleSelectionView,
};

/// Result of user interactions during rendering
pub(super) struct InteractionResult {
    pub clicked_item: Option<usize>,
    pub double_clicked_item: Option<usize>,
    pub secondary_clicked_item: Option<usize>,
    pub empty_area_click: bool,
    pub empty_area_secondary_click: bool,
    pub toggled_group: Option<GroupKey>,
}

/// Renders the virtualized list content with scroll handling and prefetch.
pub(super) fn render_virtualized_content(
    ui: &mut Ui,
    ctx: &mut ListViewContext,
    ops: &mut dyn ListViewOperations,
    available_w: f32,
    row_height: f32,
    col_widths: &ColumnWidths,
) -> InteractionResult {
    let total_rows = ctx.items.len();
    let grouped = ctx.group_projection.is_grouped() && !ctx.compact;
    ctx.visible_group_paths.clear();
    let mut clicked_item = None;
    let mut double_clicked_item = None;
    let mut secondary_clicked_item = None;
    let mut secondary_clicked_empty_area = false;

    // --- MANUAL VIRTUALIZATION START ---
    let total_content_height = if grouped {
        grouped_content_height(ctx, row_height)
    } else {
        total_rows as f32 * row_height
    };
    let viewport_rect = ui.available_rect_before_wrap();
    let viewport_h = viewport_rect.height();
    let max_scroll = (total_content_height - viewport_h).max(0.0);

    // 1. Handle mouse wheel scroll (Manual Source of Truth)
    let pointer_over_viewport = ui.ctx().pointer_hover_pos().is_some_and(|pos| {
        viewport_rect.contains(pos)
            && ui
                .ctx()
                .layer_id_at(pos)
                .is_none_or(|layer| layer.order == egui::Order::Background)
    });
    let consume_scroll = pointer_over_viewport && !ctx.global_search_active;
    scroll::apply_scroll_input(ui, ctx.mut_scroll_offset_y, max_scroll, consume_scroll);

    // 2. Clamp scroll offset
    *ctx.mut_scroll_offset_y = ctx.mut_scroll_offset_y.clamp(0.0, max_scroll);

    // 2.5 KEYBOARD SCROLL SYNC: Ensure selected item is visible
    if ctx.scroll_to_selected {
        if let Some(selected_idx) = ctx.selected_item {
            if selected_idx < total_rows {
                let item_top = if grouped {
                    grouped_item_top(ctx, selected_idx, row_height)
                } else {
                    Some(selected_idx as f32 * row_height)
                };
                if let Some(item_top) = item_top {
                    let item_bottom = item_top + row_height;

                    let current_scroll_check = *ctx.mut_scroll_offset_y;

                    // Scroll up if item is above viewport
                    if item_top < current_scroll_check {
                        *ctx.mut_scroll_offset_y = item_top.max(0.0);
                    }
                    // Scroll down if item is below viewport
                    else if item_bottom > current_scroll_check + viewport_h {
                        *ctx.mut_scroll_offset_y =
                            (item_bottom - viewport_h).clamp(0.0, max_scroll);
                    }
                }
            }
        }
    }

    let target_scroll = *ctx.mut_scroll_offset_y;
    let (current_scroll, scroll_delta) =
        scroll::compute_visual_scroll(ui, target_scroll, viewport_h, max_scroll, ctx.generation);
    let is_scrolling = scroll_delta > 0.5;

    let bg_response = ui.interact(
        viewport_rect,
        ui.id().with("list_bg"),
        Sense::click_and_drag(),
    );

    let rectangle_geometry_needed =
        ctx.rectangle_selection_state.is_some() || bg_response.drag_started();
    let rectangle_metrics = if ctx.is_computer_view {
        None
    } else if grouped && rectangle_geometry_needed {
        Some(RectangleSelectionMetrics::Grouped(
            grouped_list_rectangle_metrics(ctx, row_height, available_w, total_content_height),
        ))
    } else if !grouped {
        Some(RectangleSelectionMetrics::List(ListRectangleMetrics {
            count: total_rows,
            row_height,
            content_width: available_w,
            content_height: total_content_height,
        }))
    } else {
        None
    };
    ctx.rectangle_selection_frame.begin(
        viewport_rect,
        0.0,
        current_scroll,
        0.0,
        max_scroll,
        rectangle_metrics,
    );

    // PERFORMANCE: Track scroll changes for GPU upload throttling
    if (target_scroll - *ctx.last_scroll_offset).abs() > 0.1 {
        *ctx.last_scroll_time = std::time::Instant::now();
        *ctx.last_scroll_offset = target_scroll;
    }

    // 3. Render Virtual List
    if !ctx.is_computer_view
        && ctx.rectangle_selection_state.is_none()
        && bg_response.drag_started()
    {
        if let Some(origin) = ui.input(|input| input.pointer.press_origin()) {
            ctx.rectangle_selection_frame.request_start(origin);
        }
    }

    let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(viewport_rect));
    // CLIP FIX: Intersect with parent clip rect to ensure content never
    // extends beyond the central panel bounds into the right sidebar.
    child_ui.set_clip_rect(viewport_rect.intersect(ui.clip_rect()));

    let content_min = viewport_rect.min;

    let mut toggled_group = None;
    if grouped {
        render_grouped_virtualized(
            &mut child_ui,
            ctx,
            ops,
            content_min,
            current_scroll,
            available_w,
            row_height,
            col_widths,
            viewport_h,
            &mut clicked_item,
            &mut double_clicked_item,
            &mut secondary_clicked_item,
            &mut secondary_clicked_empty_area,
            &mut toggled_group,
        );
    } else {
        render_regular_virtualized(
            &mut child_ui,
            ctx,
            ops,
            content_min,
            current_scroll,
            viewport_h,
            total_rows,
            available_w,
            row_height,
            col_widths,
            is_scrolling,
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
            grouped_list_rectangle_metrics(ctx, row_height, available_w, total_content_height),
        ));
    }

    if let Some(state) = ctx.rectangle_selection_state.filter(|state| {
        matches!(state.view, RectangleSelectionView::List) && state.generation == ctx.generation
    }) {
        crate::ui::views::rectangle_selection::paint_overlay(
            ui,
            state,
            viewport_rect,
            0.0,
            current_scroll,
        );
    }

    // 4. Custom Scrollbar with Track-Click
    if total_content_height > viewport_h {
        render_scrollbar(
            ui,
            viewport_rect,
            viewport_h,
            total_content_height,
            max_scroll,
            current_scroll,
            ctx,
        );
    }
    // --- MANUAL VIRTUALIZATION END ---

    // Prefetch and visible range tracking
    if total_rows > 0 && !grouped {
        handle_prefetch(
            ctx,
            ops,
            current_scroll,
            viewport_h,
            row_height,
            total_rows,
            is_scrolling,
        );
    }

    // Fallback global: detect secondary click on empty area if no item was clicked
    let empty_area_secondary_click = secondary_clicked_item.is_none()
        && (secondary_clicked_empty_area || bg_response.secondary_clicked());
    let empty_area_click =
        clicked_item.is_none() && double_clicked_item.is_none() && bg_response.clicked();

    InteractionResult {
        clicked_item,
        double_clicked_item,
        secondary_clicked_item,
        empty_area_click,
        empty_area_secondary_click,
        toggled_group,
    }
}

fn grouped_list_rectangle_metrics(
    ctx: &ListViewContext,
    row_height: f32,
    content_width: f32,
    content_height: f32,
) -> GroupedRectangleMetrics {
    let mut sections = Vec::new();
    let mut content_y = 0.0;
    for section in &ctx.group_projection.sections {
        content_y += GROUP_HEADER_HEIGHT;
        if !ctx.collapsed_groups.contains(&section.key) {
            sections.push(GroupedRectangleSection {
                item_indices: section.item_indices.clone(),
                origin: egui::pos2(0.0, content_y),
            });
            content_y += section.item_indices.len() as f32 * row_height;
        }
        content_y += GROUP_GAP;
    }
    let sections: std::sync::Arc<[GroupedRectangleSection]> = sections.into();
    GroupedRectangleMetrics {
        view: RectangleSelectionView::List,
        projection_identity: GroupedProjectionIdentity::new(sections.clone(), ctx.items.len()),
        sections,
        layout: GroupedRectangleLayout::List { row_height },
        item_count: ctx.items.len(),
        content_width,
        content_height,
    }
}

fn grouped_content_height(ctx: &ListViewContext, row_height: f32) -> f32 {
    ctx.group_projection
        .sections
        .iter()
        .map(|section| {
            GROUP_HEADER_HEIGHT
                + if ctx.collapsed_groups.contains(&section.key) {
                    0.0
                } else {
                    section.item_indices.len() as f32 * row_height
                }
                + GROUP_GAP
        })
        .sum()
}

fn grouped_item_top(ctx: &ListViewContext, selected_index: usize, row_height: f32) -> Option<f32> {
    let mut top = 0.0;
    for section in &ctx.group_projection.sections {
        top += GROUP_HEADER_HEIGHT;
        if ctx.collapsed_groups.contains(&section.key) {
            top += GROUP_GAP;
            continue;
        }
        if let Some(position) = section
            .item_indices
            .iter()
            .position(|index| *index == selected_index)
        {
            return Some(top + position as f32 * row_height);
        }
        top += section.item_indices.len() as f32 * row_height + GROUP_GAP;
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn render_grouped_virtualized(
    child_ui: &mut Ui,
    ctx: &mut ListViewContext,
    ops: &mut dyn ListViewOperations,
    content_min: egui::Pos2,
    current_scroll: f32,
    available_w: f32,
    row_height: f32,
    col_widths: &ColumnWidths,
    viewport_h: f32,
    clicked_item: &mut Option<usize>,
    double_clicked_item: &mut Option<usize>,
    secondary_clicked_item: &mut Option<usize>,
    secondary_clicked_empty_area: &mut bool,
    toggled_group: &mut Option<GroupKey>,
) {
    let mut content_y = 0.0;
    let mut visible_min = usize::MAX;
    let mut visible_max = 0usize;
    for section_index in 0..ctx.group_projection.sections.len() {
        let key = ctx.group_projection.sections[section_index].key.clone();
        let item_count = ctx.group_projection.sections[section_index]
            .item_indices
            .len();
        let collapsed = ctx.collapsed_groups.contains(&key);
        let header_rect = Rect::from_min_size(
            egui::pos2(content_min.x, content_min.y + content_y - current_scroll),
            egui::vec2(available_w, GROUP_HEADER_HEIGHT),
        );
        if child_ui.is_rect_visible(header_rect)
            && render_group_header(child_ui, header_rect, &key, item_count, collapsed).clicked()
        {
            *toggled_group = Some(key.clone());
        }
        content_y += GROUP_HEADER_HEIGHT;

        if !collapsed {
            let first =
                (((current_scroll - content_y) / row_height).floor() as isize - 3).max(0) as usize;
            let last = (((current_scroll + viewport_h - content_y) / row_height).ceil() as isize
                + 3)
            .max(0) as usize;
            for position in first..last.min(item_count) {
                let index = ctx.group_projection.sections[section_index].item_indices[position];
                let Some(item) = ctx.items.get(index) else {
                    continue;
                };
                let item_rect = Rect::from_min_size(
                    egui::pos2(
                        content_min.x,
                        content_min.y + content_y + position as f32 * row_height - current_scroll,
                    ),
                    egui::vec2(available_w, row_height),
                );
                if child_ui.is_rect_visible(item_rect) {
                    visible_min = visible_min.min(index);
                    visible_max = visible_max.max(index);
                    ctx.visible_group_paths.push(item.path.clone());
                }
                render_list_item(
                    child_ui,
                    index,
                    item,
                    item_rect,
                    ctx,
                    ops,
                    clicked_item,
                    double_clicked_item,
                    secondary_clicked_item,
                    secondary_clicked_empty_area,
                    col_widths,
                    row_height,
                );
            }
            content_y += item_count as f32 * row_height;
        }
        content_y += GROUP_GAP;
    }
    *ctx.visible_index_range = (visible_min != usize::MAX).then_some((visible_min, visible_max));
}

/// Regular virtualized rendering with adaptive overscan
#[allow(clippy::too_many_arguments)]
fn render_regular_virtualized(
    child_ui: &mut Ui,
    ctx: &mut ListViewContext,
    ops: &mut dyn ListViewOperations,
    content_min: egui::Pos2,
    current_scroll: f32,
    viewport_h: f32,
    total_rows: usize,
    available_w: f32,
    row_height: f32,
    col_widths: &ColumnWidths,
    is_scrolling: bool,
    clicked_item: &mut Option<usize>,
    double_clicked_item: &mut Option<usize>,
    secondary_clicked_item: &mut Option<usize>,
    secondary_clicked_empty_area: &mut bool,
) {
    let overscan = if is_scrolling { 2 } else { 5 };
    let vis_min_row = ((current_scroll / row_height).floor() as usize).saturating_sub(overscan);
    let vis_max_row = (((current_scroll + viewport_h) / row_height).ceil() as usize) + overscan;
    let vis_max_row = vis_max_row.min(total_rows);

    // PERFORMANCE: During fast scroll, reduce overscan to minimize rendering work
    const SCROLL_RENDER_OVERSCAN: usize = 1;
    let effective_min_row = if is_scrolling {
        vis_min_row.saturating_add(overscan.saturating_sub(SCROLL_RENDER_OVERSCAN))
    } else {
        vis_min_row
    };
    let effective_max_row = if is_scrolling {
        (vis_max_row.saturating_sub(overscan.saturating_sub(SCROLL_RENDER_OVERSCAN)))
            .min(total_rows)
    } else {
        vis_max_row
    };

    for i in effective_min_row..effective_max_row {
        let item = &ctx.items[i];
        let item_rect = Rect::from_min_size(
            egui::pos2(
                content_min.x,
                content_min.y + (i as f32 * row_height) - current_scroll,
            ),
            egui::vec2(available_w, row_height),
        );

        render_list_item(
            child_ui,
            i,
            item,
            item_rect,
            ctx,
            ops,
            clicked_item,
            double_clicked_item,
            secondary_clicked_item,
            secondary_clicked_empty_area,
            col_widths,
            row_height,
        );
    }
}

/// Renders the custom scrollbar with track-click and drag support
fn render_scrollbar(
    ui: &mut Ui,
    viewport_rect: Rect,
    viewport_h: f32,
    total_content_height: f32,
    max_scroll: f32,
    current_scroll: f32,
    ctx: &mut ListViewContext,
) {
    if viewport_h <= 0.0 || total_content_height <= 0.0 || max_scroll <= 0.0 {
        return;
    }

    // Floating scrollbar dimensions (matches egui ScrollStyle config)
    const RESTING_W: f32 = 3.0;
    const HOVER_W: f32 = 8.0;
    const MARGIN: f32 = 2.0;

    // Wide interaction zone so hover detection is easy
    let interact_rect = Rect::from_min_max(
        egui::pos2(
            viewport_rect.right() - HOVER_W - MARGIN * 2.0,
            viewport_rect.top(),
        ),
        egui::pos2(viewport_rect.right(), viewport_rect.bottom()),
    );

    let scroll_id = ui.id().with("list_scrollbar");
    let response = ui.interact(interact_rect, scroll_id, Sense::click_and_drag());

    let is_hovered = response.hovered() || response.dragged();
    // Pointer anywhere in viewport = "active" state (show handle like egui's ScrollArea)
    let pointer_in_viewport = ui.input(|i| {
        i.pointer
            .hover_pos()
            .map(|p| viewport_rect.contains(p))
            .unwrap_or(false)
    });
    let bar_w = if is_hovered { HOVER_W } else { RESTING_W };

    let scroll_bar_rect = Rect::from_min_max(
        egui::pos2(viewport_rect.right() - bar_w - MARGIN, viewport_rect.top()),
        egui::pos2(viewport_rect.right() - MARGIN, viewport_rect.bottom()),
    );

    let handle_h = (viewport_h / total_content_height * viewport_h)
        .max(30.0)
        .min(viewport_h.max(30.0));
    let travel = (viewport_h - handle_h).max(1.0);
    let handle_top = (current_scroll / max_scroll) * travel;
    let handle_rect = Rect::from_min_size(
        egui::pos2(scroll_bar_rect.left(), viewport_rect.top() + handle_top),
        egui::vec2(bar_w, handle_h),
    );

    if response.clicked() {
        if let Some(click_pos) = ui.input(|i| i.pointer.interact_pos()) {
            let relative_y = click_pos.y - scroll_bar_rect.top();
            let target_handle_top = relative_y - (handle_h / 2.0);
            let scroll_ratio = target_handle_top / travel;
            *ctx.mut_scroll_offset_y = (scroll_ratio * max_scroll).clamp(0.0, max_scroll);
        }
    } else if response.dragged() {
        let delta = response.drag_delta().y;
        let scroll_per_pixel = max_scroll / travel;
        *ctx.mut_scroll_offset_y += delta * scroll_per_pixel;
        *ctx.mut_scroll_offset_y = ctx.mut_scroll_offset_y.clamp(0.0, max_scroll);
    }

    // Track background — use egui's configured scroll style opacities
    let scroll_style = &ui.style().spacing.scroll;
    let bg_opacity = if response.dragged() || response.hovered() {
        scroll_style.interact_background_opacity
    } else {
        scroll_style.dormant_background_opacity
    };
    if bg_opacity > 0.0 {
        ui.painter().rect_filled(
            scroll_bar_rect,
            4.0,
            Color32::from_black_alpha((bg_opacity * 255.0) as u8),
        );
    }

    // Handle — use egui's exact scroll handle color and opacity
    let handle_opacity = if response.dragged() || is_hovered {
        scroll_style.interact_handle_opacity
    } else if pointer_in_viewport {
        scroll_style.active_handle_opacity
    } else {
        scroll_style.dormant_handle_opacity
    };

    // Animate opacity for smooth transitions
    let opacity_id = ui.id().with("list_scrollbar_opacity");
    let dt = ui.input(|i| i.predicted_dt).min(0.05);
    let opacity = ui.ctx().data_mut(|d| {
        let current = d.get_temp_mut_or_insert_with::<f32>(opacity_id, || 0.0_f32);
        let speed = if handle_opacity > *current { 12.0 } else { 6.0 };
        *current += (handle_opacity - *current) * (dt * speed).min(1.0);
        if (*current - handle_opacity).abs() < 0.01 {
            *current = handle_opacity;
        }
        *current
    });

    if opacity > 0.005 {
        let base_color = ui.visuals().widgets.inactive.fg_stroke.color;
        let handle_color = Color32::from_rgba_unmultiplied(
            base_color.r(),
            base_color.g(),
            base_color.b(),
            (opacity * 255.0) as u8,
        );
        ui.painter()
            .rect_filled(handle_rect, bar_w / 2.0, handle_color);
    }

    if (opacity - handle_opacity).abs() > 0.01 {
        ui.ctx().request_repaint();
    }
}

/// Handles prefetch of thumbnails for items near the viewport
fn handle_prefetch(
    ctx: &mut ListViewContext,
    _ops: &mut dyn ListViewOperations,
    current_scroll: f32,
    viewport_h: f32,
    row_height: f32,
    total_rows: usize,
    _is_scrolling: bool,
) {
    let first_visible_index = (current_scroll / row_height).floor() as usize;
    let last_visible_index = ((current_scroll + viewport_h) / row_height).ceil() as usize;
    let first_visible_index = first_visible_index.min(total_rows.saturating_sub(1));
    let last_visible_index = last_visible_index.min(total_rows).saturating_sub(1);

    // Export visible range for GPU upload prioritization
    *ctx.visible_index_range = Some((first_visible_index, last_visible_index));
}
