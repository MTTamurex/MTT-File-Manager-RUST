use super::{GridViewContext, GridViewOperations};

const SCROLL_LOD_IDLE_PREFETCH_MAX_PER_FRAME: usize = 8;
const MAX_THUMBNAIL_REQUESTS_PER_FRAME: usize = 24;

pub(super) fn flush_pending_operations(
    ctx: &mut GridViewContext,
    ops: &mut dyn GridViewOperations,
) {
    for (path, size, index, modified) in ctx.pending_ops.thumbnail_loads.drain(..) {
        if let Some(index) = index {
            ops.request_thumbnail_load_with_index(path, size, index, modified);
        } else {
            ops.request_thumbnail_load(path, size, modified);
        }
    }
    for path in ctx.pending_ops.folder_scans.drain(..) {
        ops.request_folder_scan(path);
    }
    for (path, size_px) in ctx.pending_ops.folder_preview_loads.drain(..) {
        ops.request_folder_preview_load(path, size_px);
    }
    for path in ctx.pending_ops.icon_loads.drain(..) {
        ops.request_icon_load(path);
    }
    for rename_idx in ctx.pending_ops.renames.drain(..) {
        ops.rename_with_shell(rename_idx);
    }
}

pub(super) fn process_visible_range_prefetch(
    ctx: &mut GridViewContext,
    cols: usize,
    visible_rows_range: Option<(usize, usize)>,
    is_scrolling: bool,
    ops: &mut dyn GridViewOperations,
) {
    if let Some((vis_min, vis_max)) = visible_rows_range {
        let count = ctx.items.len();
        if count > 0 {
            let first_visible_index = (vis_min * cols).min(count.saturating_sub(1));
            let last_visible_index = (vis_max * cols).min(count).saturating_sub(1);

            *ctx.visible_index_range = Some((first_visible_index, last_visible_index));

            warm_scroll_lod_adjacent_rows_when_idle(ctx, cols, vis_min, vis_max, is_scrolling, ops);
        }
    }
}

fn warm_scroll_lod_adjacent_rows_when_idle(
    ctx: &mut GridViewContext,
    cols: usize,
    vis_min: usize,
    vis_max: usize,
    is_scrolling: bool,
    ops: &mut dyn GridViewOperations,
) {
    if !ctx.low_res_thumbnails_while_scrolling || is_scrolling || ctx.is_recycle_bin_view {
        return;
    }

    if ctx.thumbnail_requests_this_frame >= MAX_THUMBNAIL_REQUESTS_PER_FRAME {
        return;
    }

    let count = ctx.items.len();
    if count == 0 || cols == 0 {
        return;
    }

    let rows = ctx.prefetch_rows.min(1).max(1);
    let forward_start = vis_max;
    let forward_end = forward_start.saturating_add(rows);
    let backward_end = vis_min;
    let backward_start = backward_end.saturating_sub(rows);

    let mut requested = 0usize;
    let warmup_size = crate::ui::theme::THUMBNAIL_MIN as u32;

    for row_range in [(forward_start, forward_end), (backward_start, backward_end)] {
        for row in row_range.0..row_range.1 {
            let start = row.saturating_mul(cols);
            if start >= count {
                break;
            }
            let end = start.saturating_add(cols).min(count);
            for idx in start..end {
                if requested >= SCROLL_LOD_IDLE_PREFETCH_MAX_PER_FRAME {
                    return;
                }
                if ctx.thumbnail_requests_this_frame >= MAX_THUMBNAIL_REQUESTS_PER_FRAME {
                    return;
                }
                if ctx.loading_set.len() >= crate::ui::cache::MAX_THUMBNAIL_LOADING_SET_ITEMS {
                    return;
                }

                let item = &ctx.items[idx];
                if item.is_dir || !item.is_media() {
                    continue;
                }
                if ctx.texture_cache.peek(&item.path).is_some()
                    || ctx.loading_set.contains(&item.path)
                    || ctx.pending_upload_set.contains(&item.path)
                    || ctx.failed_thumbnails.contains(&item.path)
                {
                    continue;
                }

                ctx.loading_set.insert(item.path.clone());
                ctx.thumbnail_requests_this_frame += 1;
                requested += 1;
                ops.request_thumbnail_prefetch_with_index(
                    item.path.clone(),
                    warmup_size,
                    idx,
                    item.modified,
                );
            }
        }
    }
}
