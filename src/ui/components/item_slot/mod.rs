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

use std::borrow::Cow;

/// Returns a translated display name for special folders (OneDrive Desktop, Documents, etc.),
/// or the original filesystem name if not a special folder.
pub(crate) fn display_name_for_item(item: &FileEntry) -> Cow<'_, str> {
    if item.is_dir {
        if let Some(translated) = crate::infrastructure::onedrive::special_folder_display_name(&item.path) {
            return Cow::Owned(translated);
        }
    }
    Cow::Borrowed(&item.name)
}

/// Trait for operations needed to render an item slot
pub trait ItemSlotOperations {
    /// Requests thumbnail loading
    /// `modified`: file modification time (seconds since epoch) from folder enumeration.
    /// Pass 0 if unknown (worker will fall back to metadata syscall).
    fn request_thumbnail_load(
        &mut self,
        path: std::path::PathBuf,
        size: u32,
        directory_index: Option<usize>,
        modified: u64,
    );
    /// Requests folder scan
    fn request_folder_scan(&mut self, path: std::path::PathBuf);
    /// Requests native folder preview loading (sandwich effect)
    fn request_folder_preview_load(&mut self, path: std::path::PathBuf);
    /// Requests async icon loading (e.g.: .exe)
    fn request_icon_load(&mut self, path: std::path::PathBuf);
    /// Executes rename
    fn rename_item(&mut self, idx: usize);
}

/// Context for item slot rendering
pub struct ItemSlotContext<'a> {
    /// The item to be rendered
    pub item: &'a FileEntry,
    /// Item index in the list
    pub idx: usize,
    /// Thumbnail size
    pub thumbnail_size: f32,
    /// Whether renaming is active
    pub is_renaming: bool,
    /// Rename text (if applicable)
    pub renaming_text: Option<&'a mut String>,
    /// Whether to focus the rename input
    pub focus_rename: bool,
    /// Whether in Recycle Bin view (avoids heavy IO and thumbnails)
    pub is_recycle_bin_view: bool,
    /// Cache de texturas (LRU)
    pub texture_cache: &'a mut lru::LruCache<std::path::PathBuf, egui::TextureHandle>,
    /// Icon loader (PERSISTENT - do not create a new one each call!)
    pub icon_loader: &'a mut IconLoader,
    /// Set of scanned folders
    pub scanned_folders: &'a mut lru::LruCache<std::path::PathBuf, ()>,
    /// Set of items loading (file thumbnails)
    pub loading_set: &'a mut FxHashSet<std::path::PathBuf>,
    /// Set of items loading icons (e.g.: .exe)
    pub loading_icons: &'a mut FxHashSet<std::path::PathBuf>,
    /// Set of icons that failed (prevents infinite retry)
    pub failed_icons: &'a lru::LruCache<std::path::PathBuf, ()>,
    /// Folder preview cache (Native Sandwich)
    pub folder_preview_cache: &'a mut lru::LruCache<std::path::PathBuf, egui::TextureHandle>,
    /// Set of folders loading native preview
    pub folder_preview_loading: &'a mut FxHashSet<std::path::PathBuf>,
    /// Skip media discovery/cover scan for folder previews in special directories.
    pub skip_folder_media_reads: bool,
    /// Paths that failed thumbnail generation (LRU bounded)
    pub failed_thumbnails: &'a lru::LruCache<std::path::PathBuf, ()>,
    /// Set of items awaiting GPU upload
    pub pending_upload_set: &'a mut FxHashSet<std::path::PathBuf>,
    pub is_dense_mode: bool,
    pub is_scrolling: bool,
    /// Per-frame cap to prevent burst of thumbnail requests on folder entry
    pub thumbnail_requests_this_frame: &'a mut usize,
}

/// Renders an item slot for grid view
pub fn render_item_slot<O: ItemSlotOperations>(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    ctx: &mut ItemSlotContext,
    ops: &mut O,
) {
    if let Some(drive_info) = &ctx.item.drive_info {
        render_drive_slot(ui, rect, ctx, ops, drive_info);
    } else if ctx.item.is_dir && !ctx.item.is_archive() {
        render_directory_slot(ui, rect, ctx, ops);
    } else {
        render_file_slot(ui, rect, ctx, ops);
    }
}
