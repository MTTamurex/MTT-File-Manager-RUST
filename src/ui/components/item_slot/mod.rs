//! Item slot rendering for grid view.
//!
//! This module contains the rendering logic for individual items in grid view.

use crate::domain::file_entry::{FileEntry, SyncStatus};
// PERFORMANCE: Use FxHashSet for PathBuf keys - faster hashing
use crate::ui::cache::FxHashSet;
use crate::ui::icon_loader::IconLoader;
use eframe::egui;

mod badges;
mod drive_slot;
mod file_slot;
mod folder_slot;

use drive_slot::render_drive_slot;
use file_slot::render_file_slot;
use folder_slot::render_directory_slot;

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
    pub scanned_folders: &'a mut lru::LruCache<std::path::PathBuf, ()>,
    /// Conjunto de itens carregando (thumbnails de arquivos)
    pub loading_set: &'a mut FxHashSet<std::path::PathBuf>,
    /// Conjunto de itens carregando ícones (ex: .exe)
    pub loading_icons: &'a mut FxHashSet<std::path::PathBuf>,
    /// Conjunto de ícones que falharam (evita retry infinito)
    pub failed_icons: &'a lru::LruCache<std::path::PathBuf, ()>,
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
    } else if ctx.item.is_dir && !ctx.item.is_archive() {
        render_directory_slot(ui, rect, ctx, ops);
    } else {
        render_file_slot(ui, rect, ctx, ops);
    }
}
