//! Icon loading functionality for the file manager.
//!
//! This module handles loading Windows shell icons for files and folders.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc;

use eframe::egui;
use lru::LruCache;
use std::num::NonZeroUsize;

use crate::domain::file_entry::IconSize;
use crate::infrastructure::windows;

/// Result from a background icon extraction thread
struct AsyncIconResult {
    key: String,
    data: Option<(Vec<u8>, u32, u32)>,
}

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
    /// Cache for extension-based icons (extension -> texture)
    /// This prevents calling SHGetFileInfoW repeatedly for common types like .txt, .pdf
    extension_cache: HashMap<String, egui::TextureHandle>,
    /// Keys currently being loaded in background threads (prevents duplicate requests)
    loading_drive_icons: HashSet<String>,
    /// Channel to receive completed icon extractions from background threads
    icon_result_rx: mpsc::Receiver<AsyncIconResult>,
    /// Sender cloned into background threads
    icon_result_tx: mpsc::Sender<AsyncIconResult>,
}

impl Default for IconLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl IconLoader {
    /// Creates a new icon loader
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            icon_cache: LruCache::new(
                NonZeroUsize::new(100).expect("icon cache size must be non-zero"),
            ), // ICON_CACHE_SIZE
            folder_icon_texture: None,
            computer_icon_texture: None,
            recycle_bin_icon_texture: None,
            drive_icon_cache: HashMap::new(),
            failed_drive_icons: HashSet::new(),
            extension_cache: HashMap::new(),
            loading_drive_icons: HashSet::new(),
            icon_result_rx: rx,
            icon_result_tx: tx,
        }
    }

    /// Poll for completed background icon extractions and upload to GPU.
    /// Call this once per frame (lightweight — just drains the channel).
    pub fn poll_async_icons(&mut self, ctx: &egui::Context) {
        let mut received_any = false;
        while let Ok(result) = self.icon_result_rx.try_recv() {
            received_any = true;
            self.loading_drive_icons.remove(&result.key);
            match result.data {
                Some((rgba_data, width, height)) => {
                    let texture = ctx.load_texture(
                        format!("async_icon_{}", &result.key),
                        egui::ColorImage::from_rgba_unmultiplied(
                            [width as usize, height as usize],
                            &rgba_data,
                        ),
                        egui::TextureOptions::LINEAR,
                    );
                    self.drive_icon_cache.insert(result.key, texture);
                }
                None => {
                    self.failed_drive_icons.insert(result.key);
                }
            }
        }
        if received_any {
            ctx.request_repaint();
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
    ///                   If true, will attempt blocking extraction (suitable for preview panel).
    pub fn get_or_load_icon_sized(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        size: IconSize,
        is_folder: bool,
        allow_blocking: bool,
    ) -> Option<egui::TextureHandle> {
        let cache_key = format!("{}_{:?}", path.to_string_lossy(), size);

        // UNIQUE ICON FILES: .exe, .lnk, .ico, .cur, .ani, .com have unique embedded icons per file.
        // For these files, check cache first, then either return None (async) or load blocking.
        if !is_folder {
            let ext_str = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .or_else(|| {
                    // Manual extension parsing fallback for paths without proper extension
                    let path_str = path.to_string_lossy();
                    path_str.rfind('.').and_then(|idx| {
                        let candidate = &path_str[idx + 1..];
                        if !candidate.contains('/') && !candidate.contains('\\') {
                            Some(candidate.to_lowercase())
                        } else {
                            None
                        }
                    })
                });

            if let Some(ref ext) = ext_str {
                if matches!(ext.as_str(), "exe" | "lnk" | "ico" | "cur" | "ani" | "com") {
                    // Check cache first - async worker may have loaded the real icon
                    if let Some(texture) = self.icon_cache.get(&cache_key) {
                        return Some(texture.clone());
                    }

                    // Also check Jumbo size cache (async worker uses Jumbo for high-quality)
                    if size != IconSize::Jumbo {
                        let jumbo_key = format!("{}_{:?}", path.to_string_lossy(), IconSize::Jumbo);
                        if let Some(texture) = self.icon_cache.get(&jumbo_key) {
                            return Some(texture.clone());
                        }
                    }

                    // PERFORMANCE: Detect virtual paths (inside archives) via string check
                    // instead of path.exists() which causes synchronous HDD reads on UI thread.
                    let path_lower = path.to_string_lossy().to_lowercase();
                    let is_virtual_path =
                        crate::domain::file_entry::path_contains_archive_segment(&path_lower);

                    if is_virtual_path {
                        // Virtual path (inside ZIP): try Shell Namespace (PIDL) for correct icon
                        if let Ok((pixels, width, height)) = windows::extract_shell_icon(path, size)
                        {
                            let texture = ctx.load_texture(
                                cache_key.clone(),
                                egui::ColorImage::from_rgba_unmultiplied(
                                    [width as usize, height as usize],
                                    &pixels,
                                ),
                                egui::TextureOptions::LINEAR,
                            );
                            let cloned = texture.clone();
                            self.icon_cache.put(cache_key, texture);
                            return Some(cloned);
                        }
                        // If PIDL extraction fails, fallback to Generic Extension Logic below...
                    } else if allow_blocking {
                        // Preview panel: blocking extraction allowed (not in scroll render loop)
                        if let Ok((pixels, width, height)) =
                            windows::extract_file_icon_by_path(path, size)
                        {
                            let texture = ctx.load_texture(
                                cache_key.clone(),
                                egui::ColorImage::from_rgba_unmultiplied(
                                    [width as usize, height as usize],
                                    &pixels,
                                ),
                                egui::TextureOptions::LINEAR,
                            );
                            let cloned = texture.clone();
                            self.icon_cache.put(cache_key, texture);
                            return Some(cloned);
                        }
                    } else {
                        // Real file on disk, non-blocking: let Async Loader handle it
                        return None;
                    }
                }
            }
        }

        // Check if path is inside an archive file (virtual path)
        // MUST check this BEFORE cache lookups to avoid returning stale generic icons
        let path_str = path.to_string_lossy();
        let is_virtual_path =
            crate::domain::file_entry::path_contains_archive_segment(&path_str.to_lowercase());

        // For virtual paths (inside ZIPs), check cache first but load with Shell API if not cached
        if is_virtual_path {
            // Check cache first (specific to this file)
            if let Some(texture) = self.icon_cache.get(&cache_key) {
                return Some(texture.clone());
            }

            // Not in cache - use Shell Namespace API (PIDL) to get correct icon
            // This works for folders, executables, and all file types inside ZIPs
            match windows::extract_shell_icon(path, size) {
                Ok((pixels, width, height)) => {
                    let texture = ctx.load_texture(
                        cache_key.clone(),
                        egui::ColorImage::from_rgba_unmultiplied(
                            [width as usize, height as usize],
                            &pixels,
                        ),
                        egui::TextureOptions::LINEAR,
                    );
                    let cloned = texture.clone();
                    self.icon_cache.put(cache_key, texture);
                    return Some(cloned);
                }
                Err(_) => {
                    // Fallback to generic extension logic below
                }
            }
        }

        // For other files (not unique icon types): check cache first
        if let Some(texture) = self.icon_cache.get(&cache_key) {
            return Some(texture.clone());
        }

        // PERFORMANCE FIX: Check Extension Cache (Memory) BEFORE hitting Windows Shell API
        // This avoids calling SHGetFileInfoW for every single .txt/.pdf/etc file.
        // We only key storage by extension+size.
        if !is_folder {
            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();
                let ext_key = format!("{}_{:?}", ext_str, size);

                if let Some(texture) = self.extension_cache.get(&ext_key) {
                    // It's a common icon! Cache it for THIS file too (to speed up LRU hits)
                    // but we can just return it.
                    // Note: We might want to put it in icon_cache to keep "hot" files hot?
                    // Actually, if we hit extension cache, we don't need to pollute LRU with file paths.
                    // But LRU removal might be tricky. Let's just return it.
                    return Some(texture.clone());
                }
            }
        }

        // PERFORMANCE FIX: NEVER call path.exists() in render loop!
        // On OneDrive, this can trigger network calls (28ms+ per file).

        let icon_result = if is_folder {
            // Folders (including virtual ones in Zips) can use the generic folder icon logic
            windows::get_file_type_icon(true, "", size)
        } else if let Some(ext) = path.extension() {
            let ext_str = ext.to_string_lossy().to_lowercase();
            // Extension-based lookup is FAST (uses registry, no file access)

            // Try load
            windows::get_file_type_icon(false, &ext_str, size)
        } else {
            // No extension - try manual parsing or generic fallback
            let path_str = path.to_string_lossy();
            let manual_ext = if let Some(idx) = path_str.rfind('.') {
                let candidate = &path_str[idx + 1..];
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

            let cloned = texture.clone();

            // Populate Extension Cache if applicable
            if !is_folder {
                if let Some(ext) = path.extension() {
                    let ext_str = ext.to_string_lossy().to_lowercase();
                    let ext_key = format!("{}_{:?}", ext_str, size);
                    self.extension_cache.insert(ext_key, texture.clone());
                }
            }

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

            let texture = ctx.load_texture("recycle_bin_icon", image, egui::TextureOptions::LINEAR);
            self.recycle_bin_icon_texture = Some(texture.clone());
            return Some(texture);
        }

        None
    }

    /// Gets or loads a drive icon (non-blocking).
    ///
    /// On first call, spawns a background thread for extraction and returns None.
    /// The emoji fallback is shown until the icon is ready. On subsequent frames,
    /// `poll_async_icons()` picks up the result and caches it.
    pub fn get_or_load_drive_icon(
        &mut self,
        _ctx: &egui::Context,
        drive_path: &str,
    ) -> Option<egui::TextureHandle> {
        if self.failed_drive_icons.contains(drive_path) {
            return None;
        }

        if let Some(icon) = self.drive_icon_cache.get(drive_path) {
            return Some(icon.clone());
        }

        // Already loading in background — wait for result
        if self.loading_drive_icons.contains(drive_path) {
            return None;
        }

        // Spawn background extraction (non-blocking)
        let key = drive_path.to_string();
        self.loading_drive_icons.insert(key.clone());
        let tx = self.icon_result_tx.clone();
        std::thread::spawn(move || {
            let data = windows::extract_drive_icon(&key, IconSize::Jumbo).ok();
            let _ = tx.send(AsyncIconResult { key, data });
        });

        None
    }

    /// Clears all icon caches
    pub fn clear(&mut self) {
        self.icon_cache.clear();
        self.drive_icon_cache.clear();
        self.failed_drive_icons.clear();
        self.folder_icon_texture = None;
        self.computer_icon_texture = None;
    }

    /// Clears drive icon caches (both successful and failed), allowing fresh extraction.
    /// Called when device events indicate drive insertion/removal.
    pub fn clear_drive_icons(&mut self) {
        self.drive_icon_cache.clear();
        self.failed_drive_icons.clear();
    }

    /// Gets or loads a native icon for a specific folder path (like OneDrive).
    ///
    /// Non-blocking: spawns background extraction on first call, returns None
    /// until ready. The fallback emoji/text is shown in the sidebar meanwhile.
    pub fn get_or_load_folder_path_icon(
        &mut self,
        _ctx: &egui::Context,
        folder_path: &str,
    ) -> Option<egui::TextureHandle> {
        let cache_key = folder_path.to_string();

        if self.failed_drive_icons.contains(&cache_key) {
            return None;
        }

        if let Some(icon) = self.drive_icon_cache.get(&cache_key) {
            return Some(icon.clone());
        }

        // Already loading in background — wait for result
        if self.loading_drive_icons.contains(&cache_key) {
            return None;
        }

        // Spawn background extraction (non-blocking)
        self.loading_drive_icons.insert(cache_key.clone());
        let tx = self.icon_result_tx.clone();
        let path_owned = folder_path.to_string();
        std::thread::spawn(move || {
            let data = windows::extract_drive_icon(&path_owned, IconSize::Jumbo).ok();
            let _ = tx.send(AsyncIconResult {
                key: cache_key,
                data,
            });
        });

        None
    }
}
