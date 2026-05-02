use super::{item_renderer, GridViewContext};
use crate::ui::cache::FxHashSet;
use eframe::egui::{self, Rect, Ui};
use rust_i18n::t;
use std::path::PathBuf;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_virtualized_grid(
    ui: &mut Ui,
    ctx: &mut GridViewContext,
    viewport_rect: Rect,
    viewport_h: f32,
    current_scroll: f32,
    total_rows: usize,
    count: usize,
    cols: usize,
    padding: f32,
    item_w: f32,
    item_h: f32,
    available_w: f32,
    virtual_cell_h: f32,
    is_scrolling: bool,
    clicked_item: &mut Option<usize>,
    double_clicked_item: &mut Option<usize>,
    secondary_clicked_item: &mut Option<usize>,
) -> Option<(usize, usize)> {
    let t_total = std::time::Instant::now();
    let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(viewport_rect));
    child_ui.set_clip_rect(viewport_rect);
    let content_min = viewport_rect.min;

    let vis_min_row = (current_scroll / virtual_cell_h).floor() as usize;
    let vis_max_row = ((current_scroll + viewport_h) / virtual_cell_h).ceil() as usize;
    let visible_rows_range = Some((vis_min_row, vis_max_row));

    cleanup_loading_set(ctx, vis_min_row, vis_max_row, total_rows, cols, count);
    let t_after_cleanup = std::time::Instant::now();

    let overscan = if ctx.frame_time_peak_ms > 80.0 && !is_scrolling {
        // Recovering from inactivity wake — minimize off-screen work
        // But allow normal overscan while scrolling to prevent pop-in
        1
    } else if is_scrolling {
        if ctx.scroll_predictor.velocity > 5.0 {
            3
        } else {
            2
        }
    } else {
        4
    };

    let loop_min_row = vis_min_row.saturating_sub(overscan);
    let loop_max_row = (vis_max_row + overscan).min(total_rows);

    if ctx.is_computer_view {
        render_computer_view_sections(
            &mut child_ui,
            ctx,
            content_min,
            viewport_h,
            current_scroll,
            cols,
            padding,
            item_w,
            item_h,
            available_w,
            virtual_cell_h,
            count,
            is_scrolling,
            clicked_item,
            double_clicked_item,
            secondary_clicked_item,
        );
    } else {
        render_standard_grid(
            &mut child_ui,
            ctx,
            content_min,
            current_scroll,
            cols,
            padding,
            item_w,
            item_h,
            virtual_cell_h,
            count,
            loop_min_row,
            loop_max_row,
            is_scrolling,
            clicked_item,
            double_clicked_item,
            secondary_clicked_item,
        );
    }

    let t_after_render = std::time::Instant::now();

    let total_ms = t_total.elapsed().as_millis();
    if total_ms > 120 {
        log::warn!(
            "[PERF-GRID-VIRTUAL] total={}ms cleanup={}ms render={}ms computer_view={} rows_loop={}..{} vis_rows={:?} items={} cols={} overscan={} scrolling={}",
            total_ms,
            t_after_cleanup.duration_since(t_total).as_millis(),
            t_after_render.duration_since(t_after_cleanup).as_millis(),
            ctx.is_computer_view,
            loop_min_row,
            loop_max_row,
            visible_rows_range,
            count,
            cols,
            overscan,
            is_scrolling,
        );
    }

    visible_rows_range
}

fn cleanup_loading_set(
    ctx: &mut GridViewContext,
    vis_min_row: usize,
    vis_max_row: usize,
    total_rows: usize,
    cols: usize,
    count: usize,
) {
    // Skip cleanup when the set is small or items are still loading in batches.
    // Raising the threshold avoids rebuilding a HashSet every frame during the
    // initial folder load burst (when loading_set grows rapidly from 0 → 200).
    if ctx.loading_set.len() <= 80 {
        return;
    }

    let cleanup_margin = 8;
    let keep_min_row = vis_min_row.saturating_sub(cleanup_margin);
    let keep_max_row = (vis_max_row + cleanup_margin).min(total_rows);

    let keep_start_idx = keep_min_row * cols;
    let keep_end_idx = (keep_max_row * cols).min(count);

    let keep_paths: FxHashSet<&PathBuf> = (keep_start_idx..keep_end_idx)
        .flat_map(|idx| {
            let item = &ctx.items[idx];
            std::iter::once(&item.path).chain(item.folder_cover.iter())
        })
        .collect();
    ctx.loading_set.retain(|path| {
        keep_paths.contains(path)
            || ctx
                .shared_visible_paths
                .as_ref()
                .is_some_and(|visible_paths| visible_paths.contains(path))
    });
}

#[allow(clippy::too_many_arguments)]
fn render_computer_view_sections(
    ui: &mut Ui,
    ctx: &mut GridViewContext,
    content_min: egui::Pos2,
    viewport_h: f32,
    current_scroll: f32,
    cols: usize,
    padding: f32,
    item_w: f32,
    item_h: f32,
    available_w: f32,
    virtual_cell_h: f32,
    _count: usize,
    is_scrolling: bool,
    clicked_item: &mut Option<usize>,
    double_clicked_item: &mut Option<usize>,
    secondary_clicked_item: &mut Option<usize>,
) {
    let mut current_y = content_min.y - current_scroll;

    // PERFORMANCE: Use pre-computed indices from view_setup (computed once on items change, not per frame)
    let local_indices = ctx.computer_local_indices;
    let network_indices = ctx.computer_network_indices;

    render_section_indices(
        ui,
        ctx,
        &t!("sidebar.local_disks"),
        local_indices,
        &mut current_y,
        content_min,
        viewport_h,
        cols,
        padding,
        item_w,
        item_h,
        available_w,
        virtual_cell_h,
        is_scrolling,
        clicked_item,
        double_clicked_item,
        secondary_clicked_item,
    );
    render_section_indices(
        ui,
        ctx,
        &t!("sidebar.network_drives"),
        network_indices,
        &mut current_y,
        content_min,
        viewport_h,
        cols,
        padding,
        item_w,
        item_h,
        available_w,
        virtual_cell_h,
        is_scrolling,
        clicked_item,
        double_clicked_item,
        secondary_clicked_item,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_section_indices(
    ui: &mut Ui,
    ctx: &mut GridViewContext,
    title: &str,
    indices: &[usize],
    start_y: &mut f32,
    content_min: egui::Pos2,
    viewport_h: f32,
    cols: usize,
    padding: f32,
    item_w: f32,
    item_h: f32,
    available_w: f32,
    virtual_cell_h: f32,
    is_scrolling: bool,
    clicked_item: &mut Option<usize>,
    double_clicked_item: &mut Option<usize>,
    secondary_clicked_item: &mut Option<usize>,
) {
    let section_count = indices.len();
    if section_count == 0 {
        return;
    }

    let header_h = 25.0;
    if *start_y + header_h > content_min.y && *start_y < content_min.y + viewport_h {
        let header_x = content_min.x + padding;
        let header_w = (available_w - padding).max(0.0);
        let header_rect = Rect::from_min_size(
            egui::pos2(header_x, *start_y),
            egui::vec2(header_w, header_h),
        );
        let mut header_ui = ui.new_child(egui::UiBuilder::new().max_rect(header_rect));
        item_renderer::render_section_header(&mut header_ui, title);
    }
    *start_y += header_h;

    let rows = (section_count as f32 / cols as f32).ceil() as usize;
    let section_h = rows as f32 * virtual_cell_h + padding;

    if *start_y + section_h > content_min.y && *start_y < content_min.y + viewport_h {
        for (section_arr_idx, &real_idx) in indices.iter().enumerate() {
            let row = section_arr_idx / cols;
            let col_idx = section_arr_idx % cols;

            let item_y = *start_y + row as f32 * virtual_cell_h + padding;
            if item_y + item_h > content_min.y && item_y < content_min.y + viewport_h {
                let x_pos = col_idx as f32 * (item_w + padding) + padding;
                let item_rect = Rect::from_min_size(
                    egui::pos2(content_min.x + x_pos, item_y),
                    egui::vec2(item_w, item_h),
                );
                let item = &ctx.items[real_idx];
                item_renderer::render_grid_item(
                    ui,
                    real_idx,
                    item,
                    item_rect,
                    ctx,
                    clicked_item,
                    double_clicked_item,
                    secondary_clicked_item,
                    is_scrolling,
                );
            }
        }
    }

    *start_y += section_h;
}

#[allow(clippy::too_many_arguments)]
fn render_standard_grid(
    ui: &mut Ui,
    ctx: &mut GridViewContext,
    content_min: egui::Pos2,
    current_scroll: f32,
    cols: usize,
    padding: f32,
    item_w: f32,
    item_h: f32,
    virtual_cell_h: f32,
    count: usize,
    loop_min_row: usize,
    loop_max_row: usize,
    is_scrolling: bool,
    clicked_item: &mut Option<usize>,
    double_clicked_item: &mut Option<usize>,
    secondary_clicked_item: &mut Option<usize>,
) {
    let t_start = std::time::Instant::now();
    let mut rendered_items = 0usize;
    let mut slow_items: Vec<(usize, &str, bool, u128)> = Vec::new();
    for row in loop_min_row..loop_max_row {
        for col in 0..cols {
            let index = row * cols + col;
            if index >= count {
                break;
            }

            let x_pos = col as f32 * (item_w + padding) + padding;
            let y_pos = content_min.y + row as f32 * virtual_cell_h + padding - current_scroll;

            let item_rect = Rect::from_min_size(
                egui::pos2(content_min.x + x_pos, y_pos),
                egui::vec2(item_w, item_h),
            );

            let t_item = std::time::Instant::now();
            item_renderer::render_grid_item(
                ui,
                index,
                &ctx.items[index],
                item_rect,
                ctx,
                clicked_item,
                double_clicked_item,
                secondary_clicked_item,
                is_scrolling,
            );
            let item_ms = t_item.elapsed().as_millis();
            if item_ms > 5 {
                slow_items.push((
                    index,
                    &ctx.items[index].name,
                    ctx.items[index].is_dir,
                    item_ms,
                ));
            }
            rendered_items += 1;
        }
    }

    let elapsed_ms = t_start.elapsed().as_millis();
    if elapsed_ms > 120 {
        log::warn!(
            "[PERF-GRID-ITEMS] total={}ms rendered_items={} row_range={}..{} cols={} scrolling={} slow_items={}",
            elapsed_ms,
            rendered_items,
            loop_min_row,
            loop_max_row,
            cols,
            is_scrolling,
            slow_items.len(),
        );
        for (idx, name, is_dir, ms) in &slow_items {
            log::warn!(
                "[PERF-GRID-ITEM] idx={} name={:?} is_dir={} time={}ms",
                idx,
                name,
                is_dir,
                ms,
            );
        }
    }
}
