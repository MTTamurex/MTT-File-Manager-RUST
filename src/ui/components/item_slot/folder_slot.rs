use super::badges::render_sync_badge;
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
    if !ctx.is_recycle_bin_view && !ctx.skip_folder_media_reads {
        // --- LAZY LOAD TRIGGER ---
        // If no cover AND not yet scanned: Trigger Scan.
        if item.folder_cover.is_none() && ctx.scanned_folders.peek(&item.path).is_none() {
            ctx.scanned_folders.put(item.path.clone(), ());
            ops.request_folder_scan(item.path.clone());
        }

        // If HAS cover (from SQLite or recent discovery) BUT texture not loaded: Load!
        if let Some(ref cover_path) = item.folder_cover {
            if !ctx.texture_cache.contains(cover_path)
                && !ctx.loading_set.contains(cover_path)
                && !ctx.failed_thumbnails.contains(cover_path)
                && ctx.loading_set.len() < 200
            {
                ctx.loading_set.insert(cover_path.clone());
                ops.request_thumbnail_load(cover_path.clone(), ctx.thumbnail_size as u32, None, 0);
            }
        }
    }

    // GEOMETRY - Increased to 0.85 for larger folder preview
    let available_h = rect.height();
    let folder_w = ctx.thumbnail_size * 0.85;
    let folder_h = folder_w * 0.85;
    let text_height = 18.0;
    let content_h = folder_h + text_height;
    let vertical_margin = ((available_h - content_h) / 2.0).max(2.0);

    // Center folder horizontally in cell
    let cell_width = rect.width();
    let x_offset = (cell_width - folder_w) / 2.0;
    let start_pos = rect.min + egui::vec2(x_offset.max(0.0), vertical_margin);
    let folder_rect = egui::Rect::from_min_size(start_pos, egui::vec2(folder_w, folder_h));

    // === FOLDER DRAWING ===

    // All normal folders use our custom composed preview (with or without media content).
    // We never prematurely clear loading state — the worker always returns a result.
    // For system folders (C:\Windows tree) and Recycle Bin, skip the preview cache
    // to avoid size jumps when the preview panel triggers an async compose.
    let native_preview = if ctx.is_recycle_bin_view || ctx.skip_folder_media_reads {
        None
    } else {
        ctx.folder_preview_cache.get(&item.path)
    };
    let is_loading = !ctx.is_recycle_bin_view && ctx.folder_preview_loading.contains(&item.path);

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
                        .get_or_load_icon(ui.ctx(), &item.path, true, true)
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
                } else {
                    // No system icon available — leave space empty (no placeholder)
                }
            } else {
                // No system icon available — leave space empty (no placeholder)
            }
        } else {
            // NORMAL FOLDER: Always request our custom composed preview.
            // Worker produces back+front+thumbnail (or back+front only if no media).
            if !is_loading {
                ops.request_folder_preview_load(item.path.clone());
            }

            // While preview is loading: show system folder icon (the final content for
            // folders without media). When the composed preview arrives it replaces this
            // directly — no spinner, no grey rect placeholder.
            if let Some(sys_icon) = ctx.icon_loader.folder_icon() {
                paint_texture_centered(ui, sys_icon.id(), sys_icon.size_vec2(), folder_rect);
            }
            // If no system icon cached yet: leave space empty — no placeholder
        }
    }

    // Render sync status badge (OneDrive) for folders
    if !ctx.is_dense_mode {
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
                    egui::RichText::new(&item.name)
                        .size(11.0)
                        .color(egui::Color32::BLACK),
                )
                .truncate(),
            );
        }
    }
}
