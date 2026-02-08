use super::badges::render_sync_badge;
use super::*;

/// Renderiza um slot de arquivo
pub(super) fn render_file_slot<O: ItemSlotOperations>(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    ctx: &mut ItemSlotContext,
    ops: &mut O,
) {
    let item = ctx.item;

    // PERFORMANCE: Use is_media() method to avoid registry lookups per frame
    let is_media_file = item.is_media();

    // Thumbnail loading para arquivos de mídia (desabilitado na Lixeira)
    if is_media_file && !ctx.is_recycle_bin_view {
        let has_texture = ctx.texture_cache.contains(&item.path);
        let is_loading = ctx.loading_set.contains(&item.path);
        let is_failed = ctx.failed_thumbnails.contains(&item.path);
        let is_pending_upload = ctx.pending_upload_set.contains(&item.path);

        if !has_texture
            && !is_loading
            && !is_failed
            && !is_pending_upload
            && ctx.loading_set.len() < 200
        {
            // MAX_CONCURRENT_LOADS (increased for performance - stale entries are cleaned by grid_view)
            ctx.loading_set.insert(item.path.clone());
            ops.request_thumbnail_load(
                item.path.clone(),
                ctx.thumbnail_size as u32,
                Some(ctx.idx),
                ctx.item.modified,
            );
        }
    }

    // Carrega ícone (sempre, servirá como fallback)
    // Na Lixeira, usa get_or_load_icon que agora suporta paths virtuais com extensão
    // PERFORMANCE: allow_blocking=false prevents UI stutter on slow icons (exe/lnk)
    let file_icon = ctx
        .icon_loader
        .get_or_load_icon(ui.ctx(), &item.path, false, false);

    // Se ícone não está cacheado E não está carregando E não falhou:
    // Dispara carregamento assíncrono (apenas para casos lentos onde allow_blocking=false retornou None)
    // NOTE: Do NOT insert into loading_icons here - request_icon_load handles it.
    // Inserting here would cause the deferred request_icon_load to skip (already in set).
    // NOTE: Also works for Recycle Bin - physical_path ($R files) contain embedded icons.
    if file_icon.is_none() {
        if !ctx.loading_icons.contains(&item.path) && ctx.failed_icons.peek(&item.path).is_none() {
            ops.request_icon_load(item.path.clone());
        }
    }

    // GEOMETRIA - reduz tamanho para caber na área com margem
    let available_h = rect.height();
    let available_w = rect.width();
    let thumb_size = (ctx.thumbnail_size - 6.0).min(available_w - 4.0); // 6px margem total
    let text_height = 18.0;
    let content_h = thumb_size + text_height;
    let vertical_margin = ((available_h - content_h) / 2.0).max(2.0);

    // Centraliza horizontalmente na área disponível
    let x_offset = (available_w - thumb_size) / 2.0;
    let start_pos = rect.min + egui::vec2(x_offset.max(0.0), vertical_margin);
    let thumb_rect = egui::Rect::from_min_size(start_pos, egui::vec2(thumb_size, thumb_size));

    // Desenha thumbnail ou ícone
    let mut drew_something = false;
    if is_media_file {
        if let Some(texture) = ctx.texture_cache.get(&item.path) {
            // Thumbnail carregado - mantém aspect ratio
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
        // Fallback para ícone do Windows ou placeholder
        ui.painter()
            .rect_filled(thumb_rect, 4.0, egui::Color32::from_gray(248));
        if let Some(icon_texture) = file_icon {
            let icon_size = thumb_size * 0.5;
            let icon_rect =
                egui::Rect::from_center_size(thumb_rect.center(), egui::vec2(icon_size, icon_size));
            ui.painter().image(
                icon_texture.id(),
                icon_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            // Se nem o ícone carregou, mostra "..." se for mídia ou ícone genérico
            let text = if is_media_file { "..." } else { "📄" };
            let font_id = if is_media_file {
                egui::FontId::proportional(thumb_size * 0.3)
            } else {
                egui::FontId::proportional(thumb_size * 0.4)
            };
            ui.painter().text(
                thumb_rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                font_id,
                egui::Color32::GRAY,
            );
        }
    }

    // Render sync status badge (OneDrive)
    if !ctx.is_dense_mode {
        render_sync_badge(ui, thumb_rect, item.sync_status);
    }

    // Aloca espaço do thumbnail
    ui.allocate_rect(thumb_rect, egui::Sense::hover());

    // Texto do nome - igual às pastas
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
                        .color(egui::Color32::BLACK),
                )
                .truncate(),
            );
        }
    }
}
