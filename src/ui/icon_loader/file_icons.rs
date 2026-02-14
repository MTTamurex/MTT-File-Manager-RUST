use std::path::Path;

use super::*;

impl IconLoader {
    /// Ensures the folder icon texture is loaded.
    pub fn ensure_folder_icon(&mut self, ctx: &egui::Context) {
        if self.folder_icon_texture.is_some() {
            return;
        }

        // Try to load native Windows folder icon.
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

    /// Gets the folder icon texture (must call ensure_folder_icon first).
    pub fn folder_icon(&self) -> Option<&egui::TextureHandle> {
        self.folder_icon_texture.as_ref()
    }

    /// Gets or loads a Windows shell icon for a file path with default size (Large).
    pub fn get_or_load_icon(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        is_folder: bool,
        allow_blocking: bool,
    ) -> Option<egui::TextureHandle> {
        self.get_or_load_icon_sized(ctx, path, IconSize::Large, is_folder, allow_blocking)
    }

    /// Gets or loads a Windows shell icon for a file path with a specific size.
    ///
    /// `allow_blocking`: If false, returns None for operations that require disk access.
    /// If true, will attempt blocking extraction (suitable for preview panel).
    pub fn get_or_load_icon_sized(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        size: IconSize,
        is_folder: bool,
        allow_blocking: bool,
    ) -> Option<egui::TextureHandle> {
        let cache_key = format!("{}_{:?}", path.to_string_lossy(), size);

        // Unique-icon file types.
        if !is_folder {
            let ext_str = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .or_else(|| {
                    // Manual extension parsing fallback for paths without proper extension.
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
                    // Check cache first - async worker may have loaded the real icon.
                    if let Some(texture) = self.icon_cache.get(&cache_key) {
                        return Some(texture.clone());
                    }

                    // Also check Jumbo size cache (async worker uses Jumbo for high-quality).
                    if size != IconSize::Jumbo {
                        let jumbo_key = format!("{}_{:?}", path.to_string_lossy(), IconSize::Jumbo);
                        if let Some(texture) = self.icon_cache.get(&jumbo_key) {
                            return Some(texture.clone());
                        }
                    }

                    // Detect virtual paths (inside archives) via string check.
                    let path_lower = path.to_string_lossy().to_lowercase();
                    let is_virtual_path =
                        crate::domain::file_entry::path_contains_archive_segment(&path_lower);

                    if is_virtual_path {
                        // Virtual path (inside ZIP): try Shell Namespace (PIDL) for correct icon.
                        if let Ok((pixels, width, height)) = windows::extract_shell_icon(path, size) {
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
                        // If PIDL extraction fails, fallback to generic extension logic below.
                    } else if allow_blocking {
                        // Preview panel: blocking extraction allowed (not in scroll render loop).
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
                        // Real file on disk, non-blocking: let async loader handle it.
                        return None;
                    }
                }
            }
        }

        // Check if path is inside an archive file (virtual path).
        // Must check this before cache lookups to avoid stale generic icons.
        let path_str = path.to_string_lossy();
        let is_virtual_path =
            crate::domain::file_entry::path_contains_archive_segment(&path_str.to_lowercase());

        if is_virtual_path {
            // Check cache first (specific to this file).
            if let Some(texture) = self.icon_cache.get(&cache_key) {
                return Some(texture.clone());
            }

            // Not in cache - use Shell Namespace API (PIDL) to get correct icon.
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
                    // Fallback to generic extension logic below.
                }
            }
        }

        // For other files (not unique icon types): check cache first.
        if let Some(texture) = self.icon_cache.get(&cache_key) {
            return Some(texture.clone());
        }

        // Check extension cache (memory) before hitting Windows Shell API.
        if !is_folder {
            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();
                let ext_key = format!("{}_{:?}", ext_str, size);

                if let Some(texture) = self.extension_cache.get(&ext_key) {
                    return Some(texture.clone());
                }
            }
        }

        let icon_result = if is_folder {
            windows::get_file_type_icon(true, "", size)
        } else if let Some(ext) = path.extension() {
            let ext_str = ext.to_string_lossy().to_lowercase();
            windows::get_file_type_icon(false, &ext_str, size)
        } else {
            // No extension - try manual parsing or generic fallback.
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

            // Populate extension cache if applicable.
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
}
