//! Icon loading functionality for the file manager.
//!
//! This module handles loading Windows shell icons for files and folders.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use eframe::egui;
use lru::LruCache;
use std::num::NonZeroUsize;

use crate::domain::file_entry::IconSize;
use crate::infrastructure::windows;

/// Manages loading and caching of Windows shell icons
pub struct IconLoader {
    /// Cache for file icons (path -> texture)
    pub icon_cache: LruCache<String, egui::TextureHandle>,
    /// Folder icon texture (cached)
    folder_icon_texture: Option<egui::TextureHandle>,
    /// Computer icon texture (cached)
    computer_icon_texture: Option<egui::TextureHandle>,
    /// Recycle bin icon texture (cached)
    recycle_bin_icon_texture: Option<egui::TextureHandle>,
    /// Drive icon cache (drive path -> texture)
    drive_icon_cache: HashMap<String, egui::TextureHandle>,
    /// Remember failed drive/shell icon attempts to avoid retrying every frame
    failed_drive_icons: HashSet<String>,
}

impl IconLoader {
    /// Creates a new icon loader
    pub fn new() -> Self {
        Self {
            icon_cache: LruCache::new(NonZeroUsize::new(100).unwrap()), // ICON_CACHE_SIZE
            folder_icon_texture: None,
            computer_icon_texture: None,
            recycle_bin_icon_texture: None,
            drive_icon_cache: HashMap::new(),
            failed_drive_icons: HashSet::new(),
        }
    }

    /// Ensures the folder icon texture is loaded
    pub fn ensure_folder_icon(&mut self, ctx: &egui::Context) {
        if self.folder_icon_texture.is_some() {
            return;
        }

        // Try to load native Windows folder icon
        if let Ok((pixels, width, height)) = windows::extract_folder_icon(IconSize::Jumbo) {
            let texture = ctx.load_texture(
                "folder_icon",
                egui::ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    &pixels,
                ),
                egui::TextureOptions::LINEAR,
            );
            self.folder_icon_texture = Some(texture);
        }
    }

    /// Gets or loads a Windows shell icon for a file path with default size (Large)
    pub fn get_or_load_icon(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        is_folder: bool,
        allow_blocking: bool,
    ) -> Option<egui::TextureHandle> {
        self.get_or_load_icon_sized(ctx, path, IconSize::Large, is_folder, allow_blocking)
    }

    /// Gets or loads a Windows shell icon for a file path with a specific size
    /// PERFORMANCE: Avoids blocking I/O by using extension-based lookup first
    ///
    /// `allow_blocking`: If false, returns None for operations that require disk access (e.g. EXEs).
    pub fn get_or_load_icon_sized(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        size: IconSize,
        is_folder: bool,
        _allow_blocking: bool,
    ) -> Option<egui::TextureHandle> {
        let cache_key = format!("{}_{:?}", path.to_string_lossy(), size);

        // UNIQUE ICON FILES: .exe, .lnk, .ico, .cur, .ani, .com have unique embedded icons per file.
        // For these files, ONLY return from cache if async worker has populated it.
        // NEVER fall back to generic icons - they must be loaded via async worker.
        if !is_folder {
            let ext_str = path.extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .or_else(|| {
                    // Manual extension parsing fallback for paths without proper extension
                    let path_str = path.to_string_lossy();
                    path_str.rfind('.').and_then(|idx| {
                        let candidate = &path_str[idx+1..];
                        if !candidate.contains('/') && !candidate.contains('\\') {
                            Some(candidate.to_lowercase())
                        } else {
                            None
                        }
                    })
                });

            if let Some(ref ext) = ext_str {
                if matches!(ext.as_str(), "exe" | "lnk" | "ico" | "cur" | "ani" | "com") {
                    // Check cache - async worker may have loaded the real icon
                    if let Some(texture) = self.icon_cache.get(&cache_key) {
                        return Some(texture.clone());
                    }
                    // Not in cache - return None to trigger async load
                    // CRITICAL: Never use generic fallback for unique icon files!
                    return None;
                }
            }
        }

        // For other files (not unique icon types): check cache first
        if let Some(texture) = self.icon_cache.get(&cache_key) {
            return Some(texture.clone());
        }

        // PERFORMANCE FIX: NEVER call path.exists() in render loop!
        // On OneDrive, this can trigger network calls (28ms+ per file).

        let icon_result = if is_folder {
            // Folders (including virtual ones in Zips) can use the generic folder icon logic
            windows::get_file_type_icon(true, "", size)
        } else if let Some(ext) = path.extension() {
            let ext_str = ext.to_string_lossy().to_lowercase();
            // Extension-based lookup is FAST (uses registry, no file access)
            windows::get_file_type_icon(false, &ext_str, size)
        } else {
            // No extension - try manual parsing or generic fallback
            let path_str = path.to_string_lossy();
            let manual_ext = if let Some(idx) = path_str.rfind('.') {
                let candidate = &path_str[idx+1..];
                if !candidate.contains('/') && !candidate.contains('\\') {
                    Some(candidate.to_lowercase())
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(ext) = manual_ext {
                windows::get_file_type_icon(false, &ext, size)
            } else {
                // No extension at all -> Generic File Icon
                windows::get_file_type_icon(false, "", size)
            }
        };

        if let Ok((pixels, width, height)) = icon_result {
            let texture = ctx.load_texture(
                cache_key.clone(),
                egui::ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    &pixels,
                ),
                egui::TextureOptions::LINEAR,
            );

            // Cache the texture
            let cloned = texture.clone();
            self.icon_cache.put(cache_key, texture);
            return Some(cloned);
        }

        None
    }

    /// Gets the folder icon texture (must call ensure_folder_icon first)
    pub fn folder_icon(&self) -> Option<&egui::TextureHandle> {
        self.folder_icon_texture.as_ref()
    }

    /// Ensures the computer icon texture is loaded
    pub fn ensure_computer_icon(&mut self, ctx: &egui::Context) {
        if self.computer_icon_texture.is_some() {
            return;
        }

        if let Ok((data, width, height)) = windows::extract_computer_icon(IconSize::Jumbo) {
            let image =
                egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &data);

            self.computer_icon_texture =
                Some(ctx.load_texture("computer_icon", image, egui::TextureOptions::LINEAR));
        }
    }

    /// Gets the computer icon texture (must call ensure_computer_icon first)
    pub fn computer_icon(&self) -> Option<&egui::TextureHandle> {
        self.computer_icon_texture.as_ref()
    }

    /// Ensures the recycle bin icon texture is loaded and returns it
    pub fn ensure_recycle_bin_icon(&mut self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        if let Some(tex) = &self.recycle_bin_icon_texture {
            return Some(tex.clone());
        }

        if let Ok((data, width, height)) = windows::extract_recycle_bin_icon(IconSize::Jumbo) {
            let image =
                egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &data);

            let texture =
                ctx.load_texture("recycle_bin_icon", image, egui::TextureOptions::LINEAR);
            self.recycle_bin_icon_texture = Some(texture.clone());
            return Some(texture);
        }

        None
    }

    /// Gets or loads a drive icon
    pub fn get_or_load_drive_icon(
        &mut self,
        ctx: &egui::Context,
        drive_path: &str,
    ) -> Option<egui::TextureHandle> {
        if self.failed_drive_icons.contains(drive_path) {
            return None;
        }

        if let Some(icon) = self.drive_icon_cache.get(drive_path) {
            return Some(icon.clone());
        }

        // Try to load real drive icon
        if let Ok((rgba_data, width, height)) =
            windows::extract_drive_icon(drive_path, IconSize::Jumbo)
        {
            let texture = ctx.load_texture(
                format!("drive_{}", drive_path),
                egui::ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    &rgba_data,
                ),
                egui::TextureOptions::LINEAR,
            );
            let cloned = texture.clone();
            self.drive_icon_cache
                .insert(drive_path.to_string(), texture);
            return Some(cloned);
        }

        // Cache failure to prevent blocking retries
        self.failed_drive_icons.insert(drive_path.to_string());

        None
    }

    /// Clears all icon caches
    pub fn clear(&mut self) {
        self.icon_cache.clear();
        self.drive_icon_cache.clear();
        self.folder_icon_texture = None;
        self.computer_icon_texture = None;
    }

    /// Gets or loads a native icon for a specific folder path (like OneDrive)
    pub fn get_or_load_folder_path_icon(
        &mut self,
        ctx: &egui::Context,
        folder_path: &str,
    ) -> Option<egui::TextureHandle> {
        let cache_key = folder_path.to_string();

        if self.failed_drive_icons.contains(&cache_key) {
            return None;
        }

        if let Some(icon) = self.drive_icon_cache.get(&cache_key) {
            return Some(icon.clone());
        }

        // Try to load native folder icon for this specific path
        if let Ok((rgba_data, width, height)) =
            windows::extract_drive_icon(folder_path, IconSize::Jumbo)
        {
            let texture = ctx.load_texture(
                format!("folder_{}", folder_path),
                egui::ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    &rgba_data,
                ),
                egui::TextureOptions::LINEAR,
            );
            let cloned = texture.clone();
            self.drive_icon_cache.insert(cache_key, texture);
            return Some(cloned);
        }

        // Cache failure to avoid repeated slow attempts
        self.failed_drive_icons.insert(folder_path.to_string());

        None
    }
}
