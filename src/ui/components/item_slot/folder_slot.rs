use super::badges::{render_sync_badge, render_tag_badge};
use super::*;

/// Paints a texture centered within `container`, preserving aspect ratio.
fn paint_texture_centered(
    ui: &egui::Ui,
    tex_id: egui::TextureId,
    tex_size: egui::Vec2,
    container: egui::Rect,
) {
    let aspect = tex_size.x / tex_size.y;
    let container_aspect = container.width() / container.height();
    let (draw_w, draw_h) = if aspect > container_aspect {
        (container.width(), container.width() / aspect)
    } else {
        (container.height() * aspect, container.height())
    };
    let offset_x = (container.width() - draw_w) / 2.0;
    let offset_y = (container.height() - draw_h) / 2.0;
    let draw_rect = egui::Rect::from_min_size(
        container.min + egui::vec2(offset_x, offset_y),
        egui::vec2(draw_w, draw_h),
    );
    ui.painter().image(
        tex_id,
        draw_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
}

/// Renders a directory slot
pub(super) fn render_directory_slot<O: ItemSlotOperations>(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    ctx: &mut ItemSlotContext,
    ops: &mut O,
) {
    let item = ctx.item;
    if ctx.allow_thumbnail_requests && !ctx.is_recycle_bin_view && !ctx.skip_folder_media_reads {
        // --- LAZY LOAD TRIGGER ---
        // If no cover AND not yet scanned: Trigger Scan.
        if item.folder_cover.is_none() && ctx.scanned_folders.peek(&item.path).is_none() {
            ctx.scanned_folders.put(item.path.clone(), ());
            ops.request_folder_scan(item.path.clone());
        }

        // Folder previews are composed and cached separately. Do not enqueue the
        // raw cover thumbnail here, or every folder creates a second upload wave.
    }

    // GEOMETRY - Increased to 0.85 for larger folder preview
    let available_h = rect.height();
    let folder_w = ctx.thumbnail_size * 0.85;
    let folder_h = folder_w * 0.85;
    let ppp = ui.ctx().pixels_per_point().max(1.0);
    let display_effective_px = (folder_w.max(1.0) * ppp).ceil() as u32;
    let display_preview_bucket =
        crate::workers::thumbnail::processing::get_bucket_size(display_effective_px);
    let scroll_lod = ctx.low_res_folder_previews_while_scrolling && ctx.is_scrolling;
    let desired_preview_bucket = if scroll_lod {
        display_preview_bucket.min(256)
    } else {
        display_preview_bucket.max(crate::ui::theme::MIN_GRID_THUMBNAIL_BUCKET)
    };
    let min_effective_size_for_bucket = match desired_preview_bucket {
        0..=128 => 1,
        129..=256 => 129,
        257..=512 => 257,
        _ => 513,
    };
    let preview_request_size_px = if scroll_lod {
        min_effective_size_for_bucket
    } else {
        display_effective_px.max(min_effective_size_for_bucket)
    };
    let text_height = 18.0;
    let content_h = folder_h + text_height;
    let vertical_margin = ((available_h - content_h) / 2.0).max(2.0);

    // Center folder horizontally in cell
    let cell_width = rect.width();
    let x_offset = (cell_width - folder_w) / 2.0;
    let start_pos = rect.min + egui::vec2(x_offset.max(0.0), vertical_margin);
    let folder_rect = egui::Rect::from_min_size(start_pos, egui::vec2(folder_w, folder_h));

    // === FOLDER DRAWING ===

    // Special folders (Documents, Pictures, Desktop, etc.) always use their native
    // Windows icon — skip the composed preview entirely so it doesn't replace them.
    let item_path_text = item.path.to_string_lossy();
    let has_registered_folder_icon = ctx.icon_loader.has_registered_folder_icon(&item_path_text);
    let registered_folder_icon = has_registered_folder_icon
        .then(|| {
            ctx.icon_loader
                .get_or_load_registered_folder_icon(ui.ctx(), &item_path_text)
        })
        .flatten();
    let is_special = has_registered_folder_icon
        || crate::infrastructure::onedrive::is_special_icon_folder(&item.path);

    if is_special {
        // Special icons are square (256x256). Use a larger, square-ish rect so they
        // match the visual weight of composed folder previews.
        let special_side = folder_w;
        let special_rect = egui::Rect::from_center_size(
            folder_rect.center(),
            egui::vec2(special_side, special_side),
        );

        // Icons are pre-loaded at startup — no placeholder needed.
        let icon = registered_folder_icon.or_else(|| {
            ctx.icon_loader
                .get_or_load_folder_path_icon(ui.ctx(), &item_path_text)
                .or_else(|| ctx.icon_loader.folder_icon().cloned())
        });
        if let Some(icon) = icon {
            paint_texture_centered(ui, icon.id(), icon.size_vec2(), special_rect);
        }
    } else {
        // Normal folders use composed previews only when a media cover was discovered.
        // We never prematurely clear loading state — the worker always returns a result.
        // For system folders (C:\Windows tree) and Recycle Bin, skip the preview cache
        // to avoid size jumps when the preview panel triggers an async compose.
        let native_preview = if ctx.is_recycle_bin_view || ctx.skip_folder_media_reads {
            None
        } else {
            ctx.folder_preview_cache.get(&item.path)
        };
        let is_loading =
            !ctx.is_recycle_bin_view && ctx.folder_preview_loading.contains(&item.path);
        let has_cover = item.folder_cover.is_some();
        let needs_bucket_refresh =
            native_preview.is_some_and(|tex| (tex.size()[0] as u32) < desired_preview_bucket);

        if has_cover && needs_bucket_refresh && ctx.allow_thumbnail_requests && !is_loading {
            const MAX_SCROLL_LOD_FOLDER_PREVIEW_REQUESTS_PER_FRAME: usize = 3;
            if !scroll_lod
                || *ctx.folder_preview_requests_this_frame
                    < MAX_SCROLL_LOD_FOLDER_PREVIEW_REQUESTS_PER_FRAME
            {
                if scroll_lod {
                    *ctx.folder_preview_requests_this_frame += 1;
                }
                ops.request_folder_preview_load(item.path.clone(), preview_request_size_px);
            }
        }

        if let Some(tex) = native_preview {
            // If we have the native preview, draw maintaining aspect ratio and centering
            paint_texture_centered(ui, tex.id(), tex.size_vec2(), folder_rect);
        } else {
            // If no native preview
            let is_virtual_path = ctx.is_recycle_bin_view
                || crate::infrastructure::windows::shell_folder::is_shell_navigation_path(
                    &item.path,
                    item.is_dir,
                );

            if is_virtual_path || ctx.skip_folder_media_reads {
                // Virtual paths (recycle bin, ZIP) or system folders (C:\Windows tree):
                // Use system folder icon directly, no preview composition.
                if let Some(sys_icon) = ctx.icon_loader.folder_icon() {
                    paint_texture_centered(ui, sys_icon.id(), sys_icon.size_vec2(), folder_rect);
                } else if is_virtual_path {
                    // Extra fallback for virtual paths: try item-specific icon
                    if let Some(icon) =
                        ctx.icon_loader
                            .get_or_load_icon(ui.ctx(), &item.path, true, false)
                    {
                        let icon_size = folder_w.min(folder_h);
                        let icon_rect = egui::Rect::from_center_size(
                            folder_rect.center(),
                            egui::vec2(icon_size, icon_size),
                        );
                        ui.painter().image(
                            icon.id(),
                            icon_rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }
                }
            } else {
                // NORMAL FOLDER: request composed preview only after cover discovery.
                // Folders without media use the shared generic folder icon below.
                if has_cover && ctx.allow_thumbnail_requests && !is_loading {
                    const MAX_SCROLL_LOD_FOLDER_PREVIEW_REQUESTS_PER_FRAME: usize = 3;
                    if !scroll_lod
                        || *ctx.folder_preview_requests_this_frame
                            < MAX_SCROLL_LOD_FOLDER_PREVIEW_REQUESTS_PER_FRAME
                    {
                        if scroll_lod {
                            *ctx.folder_preview_requests_this_frame += 1;
                        }
                        ops.request_folder_preview_load(item.path.clone(), preview_request_size_px);
                    }
                }

                // While preview is missing/loading: show the shared generic folder icon.
                if let Some(sys_icon) = ctx.icon_loader.folder_icon() {
                    paint_texture_centered(ui, sys_icon.id(), sys_icon.size_vec2(), folder_rect);
                }
            }
        }
    }

    // Render sync status badge (OneDrive) for folders
    if !ctx.is_dense_mode {
        render_tag_badge(ui, folder_rect, ctx.item_tag_ids, ctx.tag_definitions);
        render_sync_badge(ui, folder_rect, item.sync_status);
    }

    // NOTE: Allocation for interaction is handled by caller using `rect`

    // TEXT: Uses Label with truncate (same as files) to respect bounds
    let text_start_y = folder_rect.bottom() + 6.0;

    if !ctx.is_dense_mode {
        let text_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left(), text_start_y),
            egui::vec2(rect.width(), 20.0), // Fixed height for text
        );

        if ctx.is_renaming {
            if let Some(text) = &mut ctx.renaming_text {
                let response = ui.put(
                    text_rect,
                    egui::TextEdit::singleline(&mut **text)
                        .frame(true)
                        .horizontal_align(egui::Align::Center)
                        .id_source("rename_input_dir"),
                );
                response.request_focus();

                // On first focus: select all text (directories have no extension)
                if ctx.focus_rename {
                    if let Some(mut state) =
                        egui::widgets::text_edit::TextEditState::load(ui.ctx(), response.id)
                    {
                        let char_count = text.chars().count();
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(char_count),
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
                    egui::RichText::new(super::display_name_for_item(item).as_ref())
                        .size(11.0)
                        .color(crate::ui::theme::text_color(ui.visuals().dark_mode)),
                )
                .truncate(),
            );
        }
    }
}
