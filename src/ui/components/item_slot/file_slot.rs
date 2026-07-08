use super::badges::{render_sync_badge, render_tag_badge};
use super::*;
use crate::domain::file_entry::IconSize;

/// Renders a file slot
pub(super) fn render_file_slot<O: ItemSlotOperations>(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    ctx: &mut ItemSlotContext,
    ops: &mut O,
) {
    let item = ctx.item;

    // PERFORMANCE: Use is_media() method to avoid registry lookups per frame
    let is_media_file = item.is_media();

    // Thumbnail loading for media files (disabled in Recycle Bin and system folders)
    if is_media_file && !ctx.is_recycle_bin_view && !ctx.skip_folder_media_reads {
        // CRITICAL: compute the bucket EXACTLY the way `request_thumbnail_load_internal`
        // does. Previously the slot used `ctx.thumbnail_size as f32 * ppp` (preserving
        // fractional part), while internal used `(ctx.thumbnail_size as u32) as f32 * ppp`
        // (truncating first). When `thumbnail_size` had a fractional part that pushed
        // the scaled value across a bucket boundary, the slot believed it had requested
        // a higher bucket than internal actually did — every frame the slot saw
        // attempted < desired and re-issued the request, producing an infinite extraction
        // loop and continuous `ctx.load_texture` calls that leak GPU staging memory.
        //
        // OPTIMIZATION: Normally extract at bucket 512 minimum so zooming in never
        // triggers re-extraction. On memory-sensitive backends while scrolling,
        // use bucket 256 first and upgrade after scrolling stops.
        let ppp = ui.ctx().pixels_per_point().max(1.0);
        let display_request_size_px = (ctx.thumbnail_size as u32).max(1);
        let display_effective_size_px = ((display_request_size_px as f32) * ppp).ceil() as u32;
        let display_bucket =
            crate::workers::thumbnail::processing::get_bucket_size(display_effective_size_px);
        let scroll_lod = ctx.low_res_thumbnails_while_scrolling && ctx.is_scrolling;
        let desired_thumbnail_bucket = if scroll_lod {
            display_bucket.min(256)
        } else {
            display_bucket.max(crate::ui::theme::MIN_GRID_THUMBNAIL_BUCKET)
        };
        let min_effective_size_for_bucket = match desired_thumbnail_bucket {
            0..=128 => 1,
            129..=256 => 129,
            257..=512 => 257,
            _ => 513,
        };
        let min_request_size_for_bucket =
            ((min_effective_size_for_bucket as f32) / ppp).ceil() as u32;
        let request_size_px = if scroll_lod {
            min_request_size_for_bucket
        } else {
            display_request_size_px.max(min_request_size_for_bucket)
        };
        let has_texture = ctx.texture_cache.peek(&item.path).is_some();
        let attempted_bucket = ctx.attempted_thumbnail_bucket.get(&item.path).copied();
        let needs_bucket_refresh = match attempted_bucket {
            Some(b) => b < desired_thumbnail_bucket,
            None => false,
        };
        let is_loading = ctx.loading_set.contains(&item.path);
        let is_failed = ctx.failed_thumbnails.contains(&item.path);
        let is_pending_upload = ctx.pending_upload_set.contains(&item.path);

        // Queue all visible cache hits quickly; worker decode and GPU upload remain
        // separately throttled, so this does not increase per-frame upload cost.
        const MAX_THUMBNAIL_REQUESTS_PER_FRAME: usize = 96;
        const OPENGL_MAX_THUMBNAIL_REQUESTS_PER_FRAME: usize = 32;
        const OPENGL_MAX_SCROLL_LOD_THUMBNAIL_REQUESTS_PER_FRAME: usize = 16;
        let max_thumbnail_requests = if ctx.low_res_thumbnails_while_scrolling {
            if scroll_lod {
                OPENGL_MAX_SCROLL_LOD_THUMBNAIL_REQUESTS_PER_FRAME
            } else {
                OPENGL_MAX_THUMBNAIL_REQUESTS_PER_FRAME
            }
        } else {
            MAX_THUMBNAIL_REQUESTS_PER_FRAME
        };

        // Allow a request whenever the texture is missing or the cached bucket
        // is too small. Per-path cooldown in `should_throttle_thumbnail_request`
        // (2s) prevents the upload->evict->re-request feedback loop after the
        // texture cache is at capacity.
        if (!has_texture || needs_bucket_refresh)
            && ctx.allow_thumbnail_requests
            && !is_loading
            && !is_failed
            && !is_pending_upload
            && ctx.loading_set.len() < crate::ui::cache::MAX_THUMBNAIL_LOADING_SET_ITEMS
            && *ctx.thumbnail_requests_this_frame < max_thumbnail_requests
        {
            // MAX_CONCURRENT_LOADS (increased for performance - stale entries are cleaned by grid_view)
            *ctx.thumbnail_requests_this_frame += 1;
            ctx.loading_set.insert(item.path.clone());
            ops.request_thumbnail_load(
                item.path.clone(),
                request_size_px,
                Some(ctx.idx),
                ctx.item.modified,
            );
        }
    }

    // Load icon (always, serves as fallback)
    // In Recycle Bin, uses get_or_load_icon which now supports virtual paths with extension
    // PERFORMANCE: allow_blocking=false prevents UI stutter on slow icons (exe/lnk)
    // Prefer Jumbo (256×256) for high-res grid rendering; fall back to Large (48×48).
    let file_icon = ctx
        .icon_loader
        .get_or_load_icon_sized(ui.ctx(), &item.path, IconSize::Jumbo, false, false)
        .or_else(|| {
            ctx.icon_loader
                .get_or_load_icon(ui.ctx(), &item.path, false, false)
        });

    // If icon is not cached AND not loading AND not failed:
    // Triggers async loading (only for slow cases where allow_blocking=false returned None)
    // NOTE: Do NOT insert into loading_icons here - request_icon_load handles it.
    // Inserting here would cause the deferred request_icon_load to skip (already in set).
    // NOTE: Also works for Recycle Bin - physical_path ($R files) contain embedded icons.
    if file_icon.is_none()
        && !ctx.loading_icons.contains(&item.path)
        && ctx.failed_icons.peek(&item.path).is_none()
    {
        ops.request_icon_load(item.path.clone());
    }

    // GEOMETRY - reduce size to fit area with margin
    let available_h = rect.height();
    let available_w = rect.width();
    let thumb_size = (ctx.thumbnail_size - 6.0).min(available_w - 4.0); // 6px margem total
    let text_height = 18.0;
    let content_h = thumb_size + text_height;
    let vertical_margin = ((available_h - content_h) / 2.0).max(2.0);

    // Center horizontally in available area
    let x_offset = (available_w - thumb_size) / 2.0;
    let start_pos = rect.min + egui::vec2(x_offset.max(0.0), vertical_margin);
    let thumb_rect = egui::Rect::from_min_size(start_pos, egui::vec2(thumb_size, thumb_size));

    // Draw thumbnail or icon
    let mut drew_something = false;
    if is_media_file {
        if let Some(texture) = ctx.texture_cache.get(&item.path) {
            // Thumbnail loaded - maintain aspect ratio
            let tex_size = texture.size_vec2();
            let aspect = tex_size.x / tex_size.y;
            let (draw_w, draw_h) = if aspect > 1.0 {
                (thumb_size, thumb_size / aspect)
            } else {
                (thumb_size * aspect, thumb_size)
            };
            let offset_x = (thumb_size - draw_w) / 2.0;
            let offset_y = (thumb_size - draw_h) / 2.0;
            let draw_rect = egui::Rect::from_min_size(
                thumb_rect.min + egui::vec2(offset_x, offset_y),
                egui::vec2(draw_w, draw_h),
            );
            ui.painter().image(
                texture.id(),
                draw_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            drew_something = true;
        }
    }

    if !drew_something {
        // Render Windows Shell icon directly — it IS the final content for this file type.
        // No grey rect: if the icon hasn't loaded yet the space is left empty (no placeholder).
        if let Some(icon_texture) = file_icon {
            let icon_size = thumb_size * 0.5;
            let icon_rect = crate::ui::views::common::snap_rect_to_physical_pixels(
                ui.ctx(),
                egui::Rect::from_center_size(thumb_rect.center(), egui::vec2(icon_size, icon_size)),
            );
            ui.painter().image(
                icon_texture.id(),
                icon_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        // If icon not yet loaded: space stays empty — no placeholder
    }

    // Render sync status badge (OneDrive)
    if !ctx.is_dense_mode {
        render_tag_badge(ui, thumb_rect, ctx.item_tag_ids, ctx.tag_definitions);
        render_sync_badge(ui, thumb_rect, item.sync_status);
    }

    // Allocate thumbnail space
    ui.allocate_rect(thumb_rect, egui::Sense::hover());

    // Name text - same as folders
    let text_start_y = thumb_rect.bottom() + 4.0;

    if !ctx.is_dense_mode {
        let text_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left(), text_start_y),
            egui::vec2(rect.width(), 20.0),
        );

        if ctx.is_renaming {
            if let Some(text) = &mut ctx.renaming_text {
                let response = ui.put(
                    text_rect,
                    egui::TextEdit::singleline(&mut **text)
                        .frame(true)
                        .horizontal_align(egui::Align::Center)
                        .id_source("rename_input_file"),
                );
                response.request_focus();

                // On first focus: select name without extension (Windows Explorer behavior)
                if ctx.focus_rename {
                    if let Some(mut state) =
                        egui::widgets::text_edit::TextEditState::load(ui.ctx(), response.id)
                    {
                        let char_count = text.chars().count();
                        let select_end = text
                            .rfind('.')
                            .map(|byte_pos| text[..byte_pos].chars().count())
                            .filter(|&pos| pos > 0)
                            .unwrap_or(char_count);
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(select_end),
                            )));
                        state.store(ui.ctx(), response.id);
                    }
                }

                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    ops.rename_item(ctx.idx);
                }
            }
        } else {
            ui.put(
                text_rect,
                egui::Label::new(
                    egui::RichText::new(&item.name)
                        .size(11.0)
                        .color(crate::ui::theme::text_color(ui.visuals().dark_mode)),
                )
                .truncate(),
            );
        }
    }
}
