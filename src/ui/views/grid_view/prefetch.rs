use super::{GridViewContext, GridViewOperations};
use crate::ui::views::ViewportTracker;

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
    for path in ctx.pending_ops.folder_preview_loads.drain(..) {
        ops.request_folder_preview_load(path);
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
    ops: &mut dyn GridViewOperations,
) {
    if let Some((vis_min, vis_max)) = visible_rows_range {
        let count = ctx.items.len();
        if count > 0 {
            let first_visible_index = (vis_min * cols).min(count.saturating_sub(1));
            let last_visible_index = (vis_max * cols).min(count).saturating_sub(1);

            *ctx.visible_index_range = Some((first_visible_index, last_visible_index));
            let tracker = ViewportTracker {
                first_visible_index,
                last_visible_index,
                prefetch_rows: ctx.prefetch_rows,
                columns: cols,
            };
            let (prefetch_start, prefetch_end) = tracker.get_prefetch_range(count);

            for index in prefetch_start..prefetch_end {
                if index >= count {
                    break;
                }
                if tracker.is_visible(index) {
                    continue;
                }
                let item = &ctx.items[index];
                if !item.is_dir
                    && item.is_media()
                    && !ctx.texture_cache.contains(&item.path)
                    && !ctx.loading_set.contains(&item.path)
                    && !ctx.pending_upload_set.contains(&item.path)
                {
                    ctx.loading_set.insert(item.path.clone());
                    ops.request_thumbnail_prefetch_with_index(
                        item.path.clone(),
                        ctx.thumbnail_size as u32,
                        index,
                        item.modified,
                    );
                }
            }

            let mut idle_visible_items = Vec::new();
            for index in first_visible_index..=last_visible_index {
                let item = &ctx.items[index];
                if !item.is_dir {
                    idle_visible_items.push(item.path.clone());
                }
            }
            if !idle_visible_items.is_empty() {
                ops.notify_idle_visible_items(idle_visible_items);
            }
        }
    }
}
