use super::badges::render_sync_badge;
use super::*;

/// Renders a directory slot
pub(super) fn render_directory_slot<O: ItemSlotOperations>(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    ctx: &mut ItemSlotContext,
    ops: &mut O,
) {
    let item = ctx.item;
    if !ctx.is_recycle_bin_view {
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
    let has_cover = item.folder_cover.is_some();

    // Never keep/render stale folder preview when we don't have a known cover candidate.
    // This avoids showing old previews for empty folders or folders whose cover was removed.
    if !ctx.is_recycle_bin_view && !has_cover {
        ctx.folder_preview_cache.pop(&item.path);
        ctx.folder_preview_loading.remove(&item.path);
    }

    // 1. Try to use the native preview (Shell Sandwich)
    let native_preview = if ctx.is_recycle_bin_view || !has_cover {
        None
    } else {
        ctx.folder_preview_cache.get(&item.path)
    };
    let is_loading = !ctx.is_recycle_bin_view && ctx.folder_preview_loading.contains(&item.path);

    if let Some(tex) = native_preview {
        // If we have the native preview, draw maintaining aspect ratio and centering
        let tex_size = tex.size_vec2();
        let aspect = tex_size.x / tex_size.y;

        // Calculate size maintaining aspect ratio
        let (draw_w, draw_h) = if aspect > 1.0 {
            (folder_rect.width(), folder_rect.width() / aspect)
        } else {
            (folder_rect.height() * aspect, folder_rect.height())
        };

        // Center in folder_rect
        let offset_x = (folder_rect.width() - draw_w) / 2.0;
        let offset_y = (folder_rect.height() - draw_h) / 2.0;
        let draw_rect = egui::Rect::from_min_size(
            folder_rect.min + egui::vec2(offset_x, offset_y),
            egui::vec2(draw_w, draw_h),
        );

        ui.painter().image(
            tex.id(),
            draw_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    } else {
        // If no native preview
        let is_virtual_path = ctx.is_recycle_bin_view
            || crate::infrastructure::windows::shell_folder::is_shell_navigation_path(
                &item.path,
                item.is_dir,
            );

        if is_virtual_path {
            // IN RECYCLE BIN or ZIP (Virtual Paths)
            // Use System Folder Icon for these virtual folders
            ctx.icon_loader.ensure_folder_icon(ui.ctx());
            if let Some(sys_icon) = ctx.icon_loader.folder_icon() {
                let icon_size = folder_w.min(folder_h);
                let icon_rect = egui::Rect::from_center_size(
                    folder_rect.center(),
                    egui::vec2(icon_size, icon_size),
                );

                ui.put(
                    icon_rect,
                    egui::Image::new(sys_icon)
                        .fit_to_original_size(1.0)
                        .max_size(egui::vec2(icon_size, icon_size)),
                );
            } else if let Some(icon) =
                ctx.icon_loader
                    .get_or_load_icon(ui.ctx(), &item.path, true, true)
            {
                // Fallback to item-specific icon (allow_blocking=true for folders usually safe, or use false if needed)
                let icon_size = folder_w.min(folder_h);
                let icon_rect = egui::Rect::from_center_size(
                    folder_rect.center(),
                    egui::vec2(icon_size, icon_size),
                );

                ui.put(
                    icon_rect,
                    egui::Image::new(&icon).max_size(egui::vec2(icon_size, icon_size)),
                );
            } else {
                // Final Fallback for virtual paths: styled empty area
                ui.painter()
                    .rect_filled(folder_rect, 4.0, egui::Color32::from_gray(245));
            }
        } else {
            // NORMAL FOLDER: Trigger loading if not yet started
            if has_cover && !is_loading {
                ops.request_folder_preview_load(item.path.clone());
            }

            if has_cover {
                // Show loading only when we have a cover candidate and are waiting for native preview.
                let spinner_size = folder_rect.width().min(folder_rect.height()) * 0.3;
                let spinner_rect = egui::Rect::from_center_size(
                    folder_rect.center(),
                    egui::vec2(spinner_size, spinner_size),
                );

                // Draw light background
                ui.painter()
                    .rect_filled(folder_rect, 4.0, egui::Color32::from_gray(245));

                let time = ui.input(|i| i.time);
                let angle = (time * 3.0) as f32;

                // Draw spinner arc
                let center = spinner_rect.center();
                let radius = spinner_size / 2.0 - 2.0;
                let stroke = egui::Stroke::new(3.0, egui::Color32::from_rgb(100, 150, 220));

                // Draw an arc (rotating semi-circle)
                let points: Vec<egui::Pos2> = (0..20)
                    .map(|i| {
                        let t = i as f32 / 19.0 * std::f32::consts::PI * 1.5; // 270 graus
                        let a = angle + t;
                        egui::pos2(center.x + radius * a.cos(), center.y + radius * a.sin())
                    })
                    .collect();

                ui.painter().add(egui::Shape::line(points, stroke));

                // PERFORMANCE: Request repaint after delay instead of immediate.
                // Spinner only needs ~15 FPS to look smooth (66ms interval).
                // This prevents CPU spinning at 60+ FPS when multiple folders are loading.
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(66));
            } else {
                // No cover candidate: draw regular folder icon immediately.
                ctx.icon_loader.ensure_folder_icon(ui.ctx());
                if let Some(sys_icon) = ctx.icon_loader.folder_icon() {
                    let icon_size = folder_w.min(folder_h);
                    let icon_rect = egui::Rect::from_center_size(
                        folder_rect.center(),
                        egui::vec2(icon_size, icon_size),
                    );

                    ui.put(
                        icon_rect,
                        egui::Image::new(sys_icon)
                            .fit_to_original_size(1.0)
                            .max_size(egui::vec2(icon_size, icon_size)),
                    );
                } else {
                    ui.painter()
                        .rect_filled(folder_rect, 4.0, egui::Color32::from_gray(245));
                }
            }
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
