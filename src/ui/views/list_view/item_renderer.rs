//! Row interactions; cell geometry and painting live in separate modules.
use super::{cell_content, ColumnWidths, ListViewContext, ListViewOperations};
use crate::domain::file_entry::FileEntry;
use eframe::egui::{self, Color32, Rect, Sense, Ui};

pub(crate) fn prepare_list_item_resources(
    ui: &Ui,
    index: usize,
    item: &FileEntry,
    ctx: &mut ListViewContext,
    ops: &mut dyn ListViewOperations,
) {
    if item.is_dir
        && !ctx.is_computer_view
        && !ctx.is_recycle_bin_view
        && item.folder_cover.is_none()
        && ctx.scanned_folders.peek(&item.path).is_none()
    {
        ctx.scanned_folders.put(item.path.clone(), ());
        ops.request_folder_scan(item.path.clone());
    }
    if item.is_dir || ctx.is_recycle_bin_view || !item.is_media() {
        return;
    }
    let selected = ctx
        .selected_file
        .is_some_and(|selected| selected.path == item.path);
    if ctx.failed_thumbnails.contains(&item.path)
        || ctx.loading_set.len() >= crate::ui::cache::MAX_THUMBNAIL_LOADING_SET_ITEMS
    {
        return;
    }
    let ppp = ui.ctx().pixels_per_point().max(1.0);
    let display_size = if ctx.show_preview_panel && selected {
        crate::domain::thumbnail::detail_preview_size(&item.path)
    } else {
        1
    };
    let bucket = crate::workers::thumbnail::processing::get_bucket_size(
        ((display_size as f32) * ppp).ceil() as u32,
    )
    .max(crate::ui::theme::MIN_GRID_THUMBNAIL_BUCKET);
    let minimum = match bucket {
        0..=128 => 1,
        129..=256 => 129,
        257..=512 => 257,
        _ => 513,
    };
    let size = display_size.max(((minimum as f32) / ppp).ceil() as u32);
    let refresh = ctx
        .attempted_thumbnail_bucket
        .peek(&item.path)
        .is_some_and(|old| *old < bucket);
    if (ctx.texture_cache.peek(&item.path).is_none() || refresh)
        && !ctx.loading_set.contains(&item.path)
        && !ctx.pending_upload_set.contains(&item.path)
        && ctx.thumbnail_requests_this_frame < 96
    {
        ctx.thumbnail_requests_this_frame += 1;
        ctx.loading_set.insert(item.path.clone());
        ops.request_thumbnail_load_with_size(item.path.clone(), size, index, item.modified);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_list_item(
    ui: &mut Ui,
    i: usize,
    stripe_row: usize,
    item: &FileEntry,
    rect: Rect,
    ctx: &mut ListViewContext,
    ops: &mut dyn ListViewOperations,
    clicked_item: &mut Option<usize>,
    double_clicked_item: &mut Option<usize>,
    secondary_clicked_item: &mut Option<usize>,
    secondary_clicked_empty_area: &mut bool,
    col_widths: &ColumnWidths,
    _row_height: f32,
) {
    prepare_list_item_resources(ui, i, item, ctx, ops);
    ui.push_id(i, |ui| {
        let opacity = if item.is_hidden { 0.5 } else { 1.0 };
        let response = ui.interact(rect, ui.id().with(i), Sense::click_and_drag());
        crate::ui::new_tab_request::item_middle_click(&response, item, ctx.is_recycle_bin_view);
        if response.clicked() {
            *clicked_item = Some(i);
        }
        if response.double_clicked() {
            *double_clicked_item = Some(i);
        }
        if response.secondary_clicked() {
            let point = ui.input(|input| input.pointer.interact_pos());
            let selected = crate::ui::views::common::effective_item_selection(
                ctx.multi_selection.contains(&item.path),
                ctx.rectangle_selection_state
                    .map(|state| state.preview_contains(i)),
            );
            let hit = point.is_some_and(|point| {
                cell_content::contains(ui, item, ctx, rect, col_widths, point)
            });
            if ctx.is_computer_view
                || crate::ui::views::common::secondary_click_targets_item(selected, hit)
            {
                *secondary_clicked_item = Some(i);
            } else {
                *secondary_clicked_empty_area = true;
            }
        }
        let (origin, point, primary_down) = ui.input(|input| {
            (
                input.pointer.press_origin(),
                input.pointer.hover_pos(),
                input.pointer.primary_down(),
            )
        });
        let drag_candidate = crate::ui::views::common::should_start_item_drag(
            response.drag_started_by(egui::PointerButton::Primary),
            response.dragged_by(egui::PointerButton::Primary),
            primary_down,
            response.is_pointer_button_down_on() && primary_down,
            origin,
            point,
        );
        let rectangle_active = ctx.rectangle_selection_state.is_some();
        if drag_candidate && !rectangle_active {
            if ctx.is_computer_view {
                *ctx.drag_started_item = Some(i);
            } else if let Some(origin) = origin {
                if cell_content::contains(ui, item, ctx, rect, col_widths, origin) {
                    *ctx.drag_started_item = Some(i);
                } else {
                    ctx.rectangle_selection_frame.request_start(origin);
                }
            } else {
                *ctx.drag_started_item = Some(i);
            }
        }
        let pointer_over = ctx
            .external_drop_pointer_pos
            .map(|point| rect.contains(point))
            .unwrap_or_else(|| response.contains_pointer() || response.hovered());
        let drag_over = ctx
            .external_drop_pointer_pos
            .map(|point| rect.contains(point))
            .unwrap_or_else(|| response.contains_pointer());
        if drag_over && item.is_dir {
            *ctx.drag_hovered_item = Some(i);
        }
        let selected = crate::ui::views::common::effective_item_selection(
            ctx.multi_selection.contains(&item.path),
            ctx.rectangle_selection_state
                .map(|state| state.preview_contains(i)),
        );
        let hovered = matches!(ctx.last_input, crate::app::state::LastInput::Mouse)
            && response.hovered()
            && !selected;
        let focused = ctx.selected_item == Some(i);
        let accent = crate::ui::theme::accent_color(ui.ctx());
        let mut visual = rect;
        visual.max.x -= 8.0;
        crate::ui::list_stripes::paint(ui, visual, stripe_row);
        if selected {
            ui.painter().rect_stroke(
                visual,
                4.0,
                egui::Stroke::new(if hovered { 2.5 } else { 2.0 }, accent),
                egui::StrokeKind::Inside,
            );
        } else if hovered || focused {
            ui.painter().rect_stroke(
                visual,
                4.0,
                egui::Stroke::new(1.0, accent.gamma_multiply(0.35)),
                egui::StrokeKind::Inside,
            );
        }
        crate::ui::clipboard_feedback::paint(ui, visual, &item.path);
        let candidate = ctx.is_item_dragging && item.is_dir && pointer_over;
        let target = ctx.is_item_dragging
            && item.is_dir
            && ctx
                .drag_target_folder
                .as_ref()
                .is_some_and(|path| *path == item.path);
        if candidate || target {
            let color = if target {
                Color32::from_rgb(24, 122, 255)
            } else {
                accent.gamma_multiply(0.75)
            };
            ui.painter().rect_stroke(
                visual.shrink(1.0),
                4.0,
                egui::Stroke::new(2.0, color),
                egui::StrokeKind::Inside,
            );
        }
        if !ctx.is_item_dragging && !rectangle_active {
            super::item_renderer_details::render_item_tooltip(
                ui,
                &response,
                item,
                ctx,
                ctx.is_recycle_bin_view,
            );
        }
        let text_color = if selected {
            crate::ui::theme::selection_text_color(ui.ctx())
        } else {
            crate::ui::theme::text_color(ui.ctx())
        }
        .gamma_multiply(opacity);
        let secondary = if selected {
            crate::ui::theme::selection_text_color(ui.ctx())
        } else {
            crate::ui::theme::secondary_text_color(ui.ctx())
        }
        .gamma_multiply(opacity);
        super::name_cell::render(
            ui, i, item, rect, col_widths, ctx, ops, &response, text_color, opacity,
        );
        if !ctx.compact {
            cell_content::render(ui, item, rect, col_widths, ctx, secondary);
        }
    });
}
