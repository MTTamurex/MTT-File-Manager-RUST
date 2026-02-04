//! Item slot rendering for grid view.
//!
//! This module contains the rendering logic for individual items in grid view.

use crate::domain::file_entry::{FileEntry, SyncStatus};
use crate::ui::icon_loader::IconLoader;
// PERFORMANCE: Use FxHashSet for PathBuf keys - faster hashing
use crate::ui::cache::FxHashSet;
use eframe::egui;

/// Trait para operações necessárias para renderizar um item slot
pub trait ItemSlotOperations {
    /// Requisita carregamento de thumbnail
    /// `modified`: file modification time (seconds since epoch) from folder enumeration.
    /// Pass 0 if unknown (worker will fall back to metadata syscall).
    fn request_thumbnail_load(
        &mut self,
        path: std::path::PathBuf,
        size: u32,
        directory_index: Option<usize>,
        modified: u64,
    );
    /// Requisita scan de pasta
    fn request_folder_scan(&mut self, path: std::path::PathBuf);
    /// Requisita carregamento de preview nativo da pasta (sandwich effect)
    fn request_folder_preview_load(&mut self, path: std::path::PathBuf);
    /// Requisita carregamento de ícone assíncrono (ex: .exe)
    fn request_icon_load(&mut self, path: std::path::PathBuf);
    /// Executa rename
    fn rename_item(&mut self, idx: usize);
}

/// Contexto para renderização de item slot
pub struct ItemSlotContext<'a> {
    /// O item a ser renderizado
    pub item: &'a FileEntry,
    /// Índice do item na lista
    pub idx: usize,
    /// Tamanho do thumbnail
    pub thumbnail_size: f32,
    /// Se está renomeando
    pub is_renaming: bool,
    /// Texto de renomeação (se aplicável)
    pub renaming_text: Option<&'a mut String>,
    /// Se deve focar no input de rename
    pub focus_rename: bool,
    /// Se estamos na view de Lixeira (evita IO pesado e thumbnails)
    pub is_recycle_bin_view: bool,
    /// Cache de texturas (LRU)
    pub texture_cache: &'a mut lru::LruCache<std::path::PathBuf, egui::TextureHandle>,
    /// Carregador de ícones (PERSISTENTE - não crie novo a cada chamada!)
    pub icon_loader: &'a mut IconLoader,
    /// Conjunto de pastas escaneadas
    pub scanned_folders: &'a mut FxHashSet<std::path::PathBuf>,
    /// Conjunto de itens carregando (thumbnails de arquivos)
    pub loading_set: &'a mut FxHashSet<std::path::PathBuf>,
    /// Conjunto de itens carregando ícones (ex: .exe)
    pub loading_icons: &'a mut FxHashSet<std::path::PathBuf>,
    /// Conjunto de ícones que falharam (evita retry infinito)
    pub failed_icons: &'a FxHashSet<std::path::PathBuf>,
    /// Cache de previews de pastas (Native Sandwich)
    pub folder_preview_cache: &'a mut lru::LruCache<std::path::PathBuf, egui::TextureHandle>,
    /// Conjunto de pastas carregando preview nativo
    pub folder_preview_loading: &'a mut FxHashSet<std::path::PathBuf>,
    /// Caminhos que falharam no thumbnail (LRU bounded)
    pub failed_thumbnails: &'a lru::LruCache<std::path::PathBuf, ()>,
    /// Conjunto de itens aguardando upload GPU
    pub pending_upload_set: &'a mut FxHashSet<std::path::PathBuf>,
    pub is_dense_mode: bool,
    pub is_scrolling: bool,
}

/// Renderiza um item slot para grid view
pub fn render_item_slot<O: ItemSlotOperations>(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    ctx: &mut ItemSlotContext,
    ops: &mut O,
) {
    if let Some(drive_info) = &ctx.item.drive_info {
        render_drive_slot(ui, rect, ctx, drive_info);
    } else if ctx.item.is_dir && !ctx.item.is_zip() {
        render_directory_slot(ui, rect, ctx, ops);
    } else {
        render_file_slot(ui, rect, ctx, ops);
    }
}

/// Renderiza um slot de drive (Este Computador)
fn render_drive_slot(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    ctx: &mut ItemSlotContext,
    drive_info: &crate::domain::file_entry::DriveInfo,
) {
    let item = ctx.item;

    // Carrega ícone real do drive
    let drive_icon = ctx
        .icon_loader
        .get_or_load_drive_icon(ui.ctx(), &item.path.to_string_lossy());

    // GEOMETRIA
    let available_h = rect.height();
    let available_w = rect.width();
    let icon_size = (ctx.thumbnail_size * 0.4).min(available_w * 0.5);
    let progress_w = (available_w * 0.8).min(150.0);
    let text_height = 36.0; // Nome + Espaço Livre
    let content_h = icon_size + 12.0 + 8.0 + text_height; // Ícone + Barra + Padding + Texto

    let vertical_margin = ((available_h - content_h) / 2.0).max(2.0);

    // Use `rect` as base for calculation instead of allocating space
    let start_y = rect.top() + vertical_margin;
    let mut current_y = start_y;

    // 1. ÍCONE
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, current_y + icon_size / 2.0),
        egui::vec2(icon_size, icon_size),
    );

    if let Some(tex) = drive_icon {
        ui.put(
            icon_rect,
            egui::Image::new(&tex)
                .max_size(egui::vec2(icon_size, icon_size))
                .maintain_aspect_ratio(true),
        );
    } else {
        ui.painter().text(
            icon_rect.center(),
            egui::Align2::CENTER_CENTER,
            "💽",
            egui::FontId::proportional(icon_size * 0.8),
            egui::Color32::GRAY,
        );
    }

    current_y += icon_size + 8.0;

    // 2. BARRA DE PROGRESSO (Espaço Usado)
    if drive_info.total_space > 0 {
        let bar_rect = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, current_y + 6.0),
            egui::vec2(progress_w, 12.0),
        );

        let used_space = drive_info.total_space - drive_info.free_space;
        let usage_ratio = used_space as f32 / drive_info.total_space as f32;

        // Cor da barra: azul ou vermelho se estiver cheio (> 90%)
        let bar_color = if usage_ratio > 0.9 {
            egui::Color32::from_rgb(230, 50, 50) // Vermelho
        } else {
            egui::Color32::from_rgb(30, 130, 230) // Azul Windows
        };

        let bg_color = egui::Color32::from_gray(230);

        ui.painter().rect_filled(bar_rect, 2.0, bg_color);

        let filled_w = progress_w * usage_ratio;
        let filled_rect = egui::Rect::from_min_size(bar_rect.min, egui::vec2(filled_w, 12.0));
        ui.painter().rect_filled(filled_rect, 2.0, bar_color);

        // Add hover interaction for the bar
        ui.interact(bar_rect, ui.id().with("drive_bar"), egui::Sense::hover());
    }

    current_y += 12.0 + 6.0;

    // 3. TEXTO (Nome e Espaço Livre)
    // Label for Name
    let name_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, current_y + 9.0), // ~half text height
        egui::vec2(progress_w, 18.0),
    );

    ui.put(
        name_rect,
        egui::Label::new(egui::RichText::new(&item.name).size(11.0).strong()).truncate(),
    );

    current_y += 18.0;

    if drive_info.total_space > 0 {
        let free_gb = drive_info.free_space as f64 / (1024.0 * 1024.0 * 1024.0);
        let total_gb = drive_info.total_space as f64 / (1024.0 * 1024.0 * 1024.0);

        let (free_val, unit) = if total_gb >= 1000.0 {
            (free_gb / 1024.0, "TB")
        } else {
            (free_gb, "GB")
        };

        let (total_val, total_unit) = if total_gb >= 1000.0 {
            (total_gb / 1024.0, "TB")
        } else {
            (total_gb, "GB")
        };

        let details_rect = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, current_y + 9.0),
            egui::vec2(progress_w, 18.0),
        );

        ui.put(
            details_rect,
            egui::Label::new(
                egui::RichText::new(format!(
                    "{:.1} {} livres de {:.1} {}",
                    free_val, unit, total_val, total_unit
                ))
                .size(9.0)
                .color(egui::Color32::from_gray(100)),
            )
            .truncate(),
        );
    }
}

/// Renderiza um slot de diretório
fn render_directory_slot<O: ItemSlotOperations>(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    ctx: &mut ItemSlotContext,
    ops: &mut O,
) {
    let item = ctx.item;

    if !ctx.is_recycle_bin_view {
        // --- GATILHO LAZY LOAD ---
        // Se não tem capa E ainda não foi escaneado: Dispara Scan.
        if item.folder_cover.is_none() && !ctx.scanned_folders.contains(&item.path) {
            ctx.scanned_folders.insert(item.path.clone());
            ops.request_folder_scan(item.path.clone());
        }

        // Se TEM capa (de SQLite ou descoberta recente) MAS a textura não está carregada: Carrega!
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

    // GEOMETRIA - Aumentado para 0.85 para folder preview maior
    let available_h = rect.height();
    let folder_w = ctx.thumbnail_size * 0.85;
    let folder_h = folder_w * 0.85;
    let text_height = 18.0;
    let content_h = folder_h + text_height;
    let vertical_margin = ((available_h - content_h) / 2.0).max(2.0);

    // Centraliza a pasta horizontalmente na celula
    let cell_width = rect.width();
    let x_offset = (cell_width - folder_w) / 2.0;
    let start_pos = rect.min + egui::vec2(x_offset.max(0.0), vertical_margin);
    let folder_rect = egui::Rect::from_min_size(start_pos, egui::vec2(folder_w, folder_h));

    // === DESENHO DA PASTA ===
    // 1. Tenta usar o preview nativo (Shell Sandwich)
    let native_preview = if ctx.is_recycle_bin_view {
        None
    } else {
        ctx.folder_preview_cache.get(&item.path)
    };
    let is_loading = !ctx.is_recycle_bin_view && ctx.folder_preview_loading.contains(&item.path);

    if let Some(tex) = native_preview {
        // Se temos o preview nativo, desenha mantendo aspect ratio e centralizando
        let tex_size = tex.size_vec2();
        let aspect = tex_size.x / tex_size.y;

        // Calcula tamanho mantendo aspect ratio
        let (draw_w, draw_h) = if aspect > 1.0 {
            (folder_rect.width(), folder_rect.width() / aspect)
        } else {
            (folder_rect.height() * aspect, folder_rect.height())
        };

        // Centraliza no folder_rect
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
        // Se não tem preview nativo
        let is_virtual_path = ctx.is_recycle_bin_view
            || crate::infrastructure::windows::shell_folder::is_shell_navigation_path(
                &item.path,
                item.is_dir,
            );

        if is_virtual_path {
            // NA LIXEIRA ou ZIP (Paths Virtuais)
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
                // Fallback para ícone específico do item (allow_blocking=true for folders usually safe, or use false if needed)
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
                // Final Fallback para virtual paths: área vazia estilizada
                ui.painter()
                    .rect_filled(folder_rect, 4.0, egui::Color32::from_gray(245));
            }
        } else {
            // PASTA NORMAL: Dispara carregamento se ainda não iniciou
            if !is_loading {
                ops.request_folder_preview_load(item.path.clone());
            }

            // SEMPRE mostra loading spinner para pastas normais sem preview
            // (NUNCA mostra ícone de pasta genérico/customizado como placeholder)
            let spinner_size = folder_rect.width().min(folder_rect.height()) * 0.3;
            let spinner_rect = egui::Rect::from_center_size(
                folder_rect.center(),
                egui::vec2(spinner_size, spinner_size),
            );

            // Desenha fundo leve
            ui.painter()
                .rect_filled(folder_rect, 4.0, egui::Color32::from_gray(245));

            let time = ui.input(|i| i.time);
            let angle = (time * 3.0) as f32;

            // Desenha arco do spinner
            let center = spinner_rect.center();
            let radius = spinner_size / 2.0 - 2.0;
            let stroke = egui::Stroke::new(3.0, egui::Color32::from_rgb(100, 150, 220));

            // Desenha um arco (semi-círculo rotativo)
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
        }
    }

    // Render sync status badge (OneDrive) for folders
    if !ctx.is_dense_mode {
        render_sync_badge(ui, folder_rect, item.sync_status);
    }

    // NOTE: Allocation for interaction is handled by caller using `rect`

    // TEXTO: Usa Label com truncate (igual aos arquivos) para respeitar limites
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
                    if let Some(mut state) = egui::widgets::text_edit::TextEditState::load(ui.ctx(), response.id) {
                        let char_count = text.chars().count();
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
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

/// Renderiza um slot de arquivo
fn render_file_slot<O: ItemSlotOperations>(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    ctx: &mut ItemSlotContext,
    ops: &mut O,
) {
    let item = ctx.item;

    // Detecta se é arquivo de mídia usando Windows Perceived Type API
    // Respeita handlers instalados (K-Lite/Icaros) - suporta OGM, MKV, etc.
    let is_media_file = item.path
        .extension()
        .map(|ext| crate::infrastructure::windows::is_media_extension(&ext.to_string_lossy()))
        .unwrap_or(false);

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
        if !ctx.loading_icons.contains(&item.path) && !ctx.failed_icons.contains(&item.path) {
            ops.request_icon_load(item.path.clone());
        }
    }

    // GEOMETRIA - reduz tamanho para caber na area com margem
    let available_h = rect.height();
    let available_w = rect.width();
    let thumb_size = (ctx.thumbnail_size - 6.0).min(available_w - 4.0); // 6px margem total
    let text_height = 18.0;
    let content_h = thumb_size + text_height;
    let vertical_margin = ((available_h - content_h) / 2.0).max(2.0);

    // Centraliza horizontalmente na area disponivel
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

    // Texto do nome - igual as pastas
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
                    if let Some(mut state) = egui::widgets::text_edit::TextEditState::load(ui.ctx(), response.id) {
                        let char_count = text.chars().count();
                        let select_end = text.rfind('.')
                            .map(|byte_pos| text[..byte_pos].chars().count())
                            .filter(|&pos| pos > 0)
                            .unwrap_or(char_count);
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
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

/// Renders a sync status badge (OneDrive) on the bottom-right corner of the thumbnail
fn render_sync_badge(ui: &mut egui::Ui, thumb_rect: egui::Rect, status: SyncStatus) {
    if status == SyncStatus::None {
        return; // No badge for normal files
    }

    let badge_size = 18.0;
    let badge_pos = egui::pos2(
        thumb_rect.right() - badge_size - 2.0,
        thumb_rect.bottom() - badge_size - 2.0,
    );
    let badge_center = badge_pos + egui::vec2(badge_size / 2.0, badge_size / 2.0);
    let badge_radius = badge_size / 2.0;

    let painter = ui.painter();

    match status {
        SyncStatus::CloudOnly => {
            // Blue cloud icon - file needs download
            painter.circle_filled(
                badge_center,
                badge_radius,
                egui::Color32::from_rgb(0, 120, 215),
            );
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "☁",
                egui::FontId::proportional(12.0),
                egui::Color32::WHITE,
            );
        }
        SyncStatus::Syncing => {
            // Blue circular arrows - file is being synced
            painter.circle_filled(
                badge_center,
                badge_radius,
                egui::Color32::from_rgb(0, 120, 215),
            );
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "⟳",
                egui::FontId::proportional(13.0),
                egui::Color32::WHITE,
            );
        }
        SyncStatus::Pinned => {
            // Green solid circle with check - always keep on device
            painter.circle_filled(
                badge_center,
                badge_radius,
                egui::Color32::from_rgb(0, 150, 0),
            );
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "✓",
                egui::FontId::proportional(11.0),
                egui::Color32::WHITE,
            );
        }
        SyncStatus::LocallyAvailable => {
            // White circle with green outline/check - downloaded on demand
            painter.circle_filled(badge_center, badge_radius, egui::Color32::WHITE);
            painter.circle_stroke(
                badge_center,
                badge_radius - 1.0,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(0, 150, 0)),
            );
            painter.text(
                badge_center,
                egui::Align2::CENTER_CENTER,
                "✓",
                egui::FontId::proportional(11.0),
                egui::Color32::from_rgb(0, 150, 0),
            );
        }
        SyncStatus::None => {} // Already handled above
    }
}
