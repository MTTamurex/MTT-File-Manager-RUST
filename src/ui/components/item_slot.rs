//! Item slot rendering for grid view.
//!
//! This module contains the rendering logic for individual items in grid view.

use eframe::egui;
use crate::domain::file_entry::FileEntry;
use crate::ui::cache::CacheManager;
use crate::ui::icon_loader::IconLoader;

/// Trait para operações necessárias para renderizar um item slot
pub trait ItemSlotOperations {
    /// Requisita carregamento de thumbnail
    fn request_thumbnail_load(&mut self, path: std::path::PathBuf);
    /// Requisita scan de pasta
    fn request_folder_scan(&mut self, path: std::path::PathBuf);
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
    /// Cache de texturas
    pub texture_cache: &'a mut CacheManager,
    /// Carregador de ícones
    pub icon_loader: &'a mut IconLoader,
    /// Conjunto de pastas escaneadas
    pub scanned_folders: &'a mut std::collections::HashSet<std::path::PathBuf>,
    /// Conjunto de itens carregando
    pub loading_set: &'a mut std::collections::HashSet<std::path::PathBuf>,
}

/// Renderiza um item slot para grid view
pub fn render_item_slot<O: ItemSlotOperations>(
    ui: &mut egui::Ui,
    ctx: &mut ItemSlotContext,
    ops: &mut O,
) {
    if ctx.item.is_dir {
        render_directory_slot(ui, ctx, ops);
    } else {
        render_file_slot(ui, ctx, ops);
    }
}

/// Renderiza um slot de diretório
fn render_directory_slot<O: ItemSlotOperations>(
    ui: &mut egui::Ui,
    ctx: &mut ItemSlotContext,
    ops: &mut O,
) {
    let item = ctx.item;
    
    // --- GATILHO LAZY LOAD ---
    // Se não tem capa E ainda não foi escaneado: Dispara Scan.
    if item.folder_cover.is_none() && !ctx.scanned_folders.contains(&item.path) {
        ctx.scanned_folders.insert(item.path.clone());
        ops.request_folder_scan(item.path.clone());
    }
    
    // GEOMETRIA
    let available_h = ui.available_height();
    let folder_w = ctx.thumbnail_size * 0.60;
    let folder_h = folder_w * 0.85;
    let text_height = 18.0;
    let content_h = folder_h + text_height;
    let vertical_margin = ((available_h - content_h) / 2.0).max(2.0);
    
    // Margem superior para centralizar verticalmente
    ui.add_space(vertical_margin);
    
    // Centraliza a pasta horizontalmente na celula
    let cell_width = ui.available_width();
    let x_offset = (cell_width - folder_w) / 2.0;
    let start_pos = ui.cursor().min + egui::vec2(x_offset.max(0.0), 0.0);
    let folder_rect = egui::Rect::from_min_size(start_pos, egui::vec2(folder_w, folder_h));

    // CORES
    let color_back = egui::Color32::from_rgb(200, 160, 50);
    let color_front = egui::Color32::from_rgb(255, 210, 70);

    // Dimensões
    let tab_h = folder_h * 0.15;
    let tab_w = folder_w * 0.40;
    let front_h = folder_h * 0.50;

    // === DESENHO 1: BASE SÓLIDA (evita qualquer gap) ===
    // Desenha TODO o corpo como uma única forma sólida
    ui.painter().rect_filled(
        egui::Rect::from_min_size(folder_rect.min, egui::vec2(tab_w, tab_h)),
        egui::CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 },
        color_back
    );
    ui.painter().rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(folder_rect.min.x, folder_rect.min.y + tab_h),
            folder_rect.max
        ),
        egui::CornerRadius { nw: 0, ne: 3, sw: 4, se: 4 },
        color_back
    );

    // === DESENHO 2: PREVIEW (com clipping para não escapar) ===
    if let Some(cover_path) = &item.folder_cover {
        if !ctx.texture_cache.has_thumbnail(cover_path) && !ctx.texture_cache.is_loading(cover_path) {
            if ctx.loading_set.len() < 30 { // MAX_CONCURRENT_LOADS
                ctx.loading_set.insert(cover_path.clone());
                ops.request_thumbnail_load(cover_path.clone());
            }
        }
    }

    if let Some(tex) = item.folder_cover.as_ref().and_then(|p| -> Option<&egui::TextureHandle> { ctx.texture_cache.get_thumbnail(p) }) {
        // Área onde o preview pode aparecer (com margens)
        let margin_x = 6.0;
        let margin_top = 4.0;
        let preview_area = egui::Rect::from_min_max(
            egui::pos2(folder_rect.min.x + margin_x, folder_rect.min.y + tab_h + margin_top),
            egui::pos2(folder_rect.max.x - margin_x, folder_rect.max.y - front_h)
        );

        let size = tex.size();
        let tex_size = egui::vec2(size[0] as f32, size[1] as f32);
        let aspect_img = tex_size.x / tex_size.y;
        let aspect_view = preview_area.width() / preview_area.height();

        let uv_rect = if aspect_img > aspect_view {
            let scale = aspect_view / aspect_img;
            let offset = (1.0 - scale) / 2.0;
            egui::Rect::from_min_max(egui::pos2(offset, 0.0), egui::pos2(1.0 - offset, 1.0))
        } else {
            let scale = aspect_img / aspect_view;
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, scale))
        };

        // Usa push_clip_rect para garantir que a imagem não escape
        ui.painter().with_clip_rect(preview_area).image(tex.id(), preview_area, uv_rect, egui::Color32::WHITE);
    }

    // === DESENHO 3: BOLSO FRONTAL (sobrepõe preview) ===
    let front_rect = egui::Rect::from_min_max(
        egui::pos2(folder_rect.min.x, folder_rect.max.y - front_h),
        folder_rect.max
    );
    ui.painter().rect_filled(front_rect, egui::CornerRadius { nw: 0, ne: 0, sw: 4, se: 4 }, color_front);

    // Borda sutil
    ui.painter().rect_stroke(
        front_rect,
        egui::CornerRadius { nw: 0, ne: 0, sw: 4, se: 4 },
        egui::Stroke::new(1.0, egui::Color32::from_rgb(200, 150, 30)),
        egui::StrokeKind::Inside
    );

    // Aloca espaço da pasta
    ui.allocate_rect(folder_rect, egui::Sense::hover());

    // TEXTO: Usa Label com truncate (igual aos arquivos) para respeitar limites
    ui.add_space(6.0);  // Gap entre pasta e texto
    
    if ctx.is_renaming {
        if let Some(text) = &mut ctx.renaming_text {
            let response = ui.add(egui::TextEdit::singleline(&mut **text)
                .frame(true)
                .horizontal_align(egui::Align::Center)
                .id_source("rename_input_dir"));
            
            if ctx.focus_rename {
                response.request_focus();
            }

            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                ops.rename_item(ctx.idx);
            } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                // Cancel rename - handled by caller
            } else if response.clicked_elsewhere() {
                // Cancel rename - handled by caller
            }
        }
    } else {
        ui.vertical_centered(|ui| {
            ui.add(egui::Label::new(
                egui::RichText::new(&item.name)
                    .size(11.0)
                    .color(egui::Color32::BLACK)
            ).truncate());
        });
    }
}

/// Renderiza um slot de arquivo
fn render_file_slot<O: ItemSlotOperations>(
    ui: &mut egui::Ui,
    ctx: &mut ItemSlotContext,
    ops: &mut O,
) {
    let item = ctx.item;
    let path_clone = item.path.clone();
    
    // Detecta se é arquivo de mídia
    let is_media_file = if let Some(ext) = path_clone.extension() {
        let ext_lower = ext.to_string_lossy().to_lowercase();
        matches!(ext_lower.as_str(),
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" |
            "tiff" | "tif" | "ico" | "heic" | "heif" | "avif" |
            "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" |
            "webm" | "m4v" | "mpg" | "mpeg" | "3gp" | "ts"
        )
    } else {
        false
    };
    
    // Thumbnail loading para arquivos de mídia
    if is_media_file {
        let has_texture = ctx.texture_cache.has_thumbnail(&path_clone);
        let is_loading = ctx.texture_cache.is_loading(&path_clone);
        
        if !has_texture && !is_loading && ctx.loading_set.len() < 30 { // MAX_CONCURRENT_LOADS
            ctx.loading_set.insert(path_clone.clone());
            ops.request_thumbnail_load(path_clone.clone());
        }
    }
    
    // Carrega ícone (sempre, servirá como fallback)
    let file_icon = ctx.icon_loader.get_or_load_icon(ui.ctx(), &path_clone);
    
    // GEOMETRIA - reduz tamanho para caber na area com margem
    let available_h = ui.available_height();
    let available_w = ui.available_width();
    let thumb_size = (ctx.thumbnail_size - 6.0).min(available_w - 4.0); // 6px margem total
    let text_height = 18.0;
    let content_h = thumb_size + text_height;
    let vertical_margin = ((available_h - content_h) / 2.0).max(2.0);
    
    // Margem superior para centralizar verticalmente
    ui.add_space(vertical_margin);
    
    // Centraliza horizontalmente na area disponivel
    let x_offset = (available_w - thumb_size) / 2.0;
    let start_pos = ui.cursor().min + egui::vec2(x_offset.max(0.0), 0.0);
    let thumb_rect = egui::Rect::from_min_size(start_pos, egui::vec2(thumb_size, thumb_size));
    
    // Desenha thumbnail ou ícone
    let mut drew_something = false;
    if is_media_file {
        if let Some(texture) = ctx.texture_cache.get_thumbnail(&path_clone) {
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
                egui::vec2(draw_w, draw_h)
            );
            ui.painter().image(texture.id(), draw_rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            drew_something = true;
        }
    }

    if !drew_something {
        // Fallback para ícone do Windows ou placeholder
        ui.painter().rect_filled(thumb_rect, 4.0, egui::Color32::from_gray(248));
        if let Some(icon_texture) = file_icon {
            let icon_size = thumb_size * 0.5;
            let icon_rect = egui::Rect::from_center_size(thumb_rect.center(), egui::vec2(icon_size, icon_size));
            ui.painter().image(icon_texture.id(), icon_rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        } else {
            // Se nem o ícone carregou, mostra "..." se for mídia ou ícone genérico
            let text = if is_media_file { "..." } else { "📄" };
            let font_id = if is_media_file { 
                egui::FontId::proportional(thumb_size * 0.3) 
            } else { 
                egui::FontId::proportional(thumb_size * 0.4)
            };
            ui.painter().text(thumb_rect.center(), egui::Align2::CENTER_CENTER, text, font_id, egui::Color32::GRAY);
        }
    }
    
    // Aloca espaço do thumbnail
    ui.allocate_rect(thumb_rect, egui::Sense::hover());
    
    // Texto do nome - igual as pastas
    ui.add_space(4.0);
    
    if ctx.is_renaming {
        if let Some(text) = &mut ctx.renaming_text {
            let response = ui.add(egui::TextEdit::singleline(&mut **text)
                .frame(true)
                .horizontal_align(egui::Align::Center)
                .id_source("rename_input_file"));
            
            if ctx.focus_rename {
                response.request_focus();
            }

            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                ops.rename_item(ctx.idx);
            } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                // Cancel rename - handled by caller
            } else if response.clicked_elsewhere() {
                // Cancel rename - handled by caller
            }
        }
    } else {
        ui.vertical_centered(|ui| {
            ui.add(egui::Label::new(
                egui::RichText::new(&item.name)
                    .size(11.0)
                    .color(egui::Color32::BLACK)
            ).truncate());
        });
    }
}
