use std::path::Path;

use super::*;

/// Build a cache key for icon_cache without per-call allocation on the hot path.
/// Only called when we actually need to access icon_cache (cache miss on extension_cache).
#[inline]
fn make_cache_key(path: &Path, size: IconSize) -> String {
    let path_text = path.to_string_lossy();
    let suffix = match size {
        IconSize::Small => "_Small",
        IconSize::Large => "_Large",
        IconSize::Jumbo => "_Jumbo",
    };
    let mut key = String::with_capacity(path_text.len() + suffix.len());
    key.push_str(path_text.as_ref());
    key.push_str(suffix);
    key
}

impl IconLoader {
    /// Sets the custom folder icon from pre-composed RGBA data.
    pub fn set_folder_icon(&mut self, ctx: &egui::Context, pixels: &[u8], width: u32, height: u32) {
        let texture = ctx.load_texture(
            "folder_icon_custom",
            egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], pixels),
            egui::TextureOptions::LINEAR,
        );
        self.folder_icon_texture = Some(texture);
    }

    /// Gets the folder icon texture (pre-set at init via `set_folder_icon`).
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
        // PERF FIX (A-3): Build cache_key lazily — only when we actually need it for
        // icon_cache insertion. For the hot path (extension cache hit), we avoid the
        // per-frame String allocation entirely.

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
                if crate::infrastructure::windows::icons::is_per_file_icon_ext(ext) {
                    let cache_key = make_cache_key(path, size);
                    // Check cache first - async worker may have loaded the real icon.
                    if let Some(texture) = self.icon_cache.get(&cache_key) {
                        return Some(texture.clone());
                    }

                    // Also check Jumbo size cache (async worker uses Jumbo for high-quality).
                    if size != IconSize::Jumbo {
                        let jumbo_key = make_cache_key(path, IconSize::Jumbo);
                        if let Some(texture) = self.icon_cache.get(&jumbo_key) {
                            return Some(texture.clone());
                        }
                    }

                    // Detect virtual paths (inside archives) via string check.
                    let path_lower = path.to_string_lossy().to_lowercase();
                    let is_virtual_path =
                        crate::domain::file_entry::path_contains_archive_segment(&path_lower);

                    if is_virtual_path {
                        // PERF FIX (A-4): Virtual path (inside ZIP) — delegate to async worker
                        // instead of blocking the UI thread with Shell Namespace calls.
                        if !allow_blocking {
                            return None;
                        }
                        // Preview panel: blocking extraction allowed.
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
                } else if crate::infrastructure::windows::icons::requires_real_file_for_shared_icon(
                    ext,
                ) && allow_blocking
                {
                    let canonical_ext =
                        crate::infrastructure::windows::icons::canonical_icon_ext(ext);
                    let ext_key = format!("{}_{:?}", canonical_ext, size);
                    if let Some(texture) = self.extension_cache.get(&ext_key) {
                        return Some(texture.clone());
                    }

                    let cache_key = make_cache_key(path, size);
                    if let Some(texture) = self.icon_cache.get(&cache_key) {
                        return Some(texture.clone());
                    }

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
                        if self.extension_cache.peek(&ext_key).is_none() {
                            self.extension_cache.put(ext_key, texture.clone());
                        }
                        self.icon_cache.put(cache_key, texture);
                        return Some(cloned);
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
            let cache_key = make_cache_key(path, size);
            // Check cache first (specific to this file).
            if let Some(texture) = self.icon_cache.get(&cache_key) {
                return Some(texture.clone());
            }

            // PERF FIX (A-4): Non-blocking callers skip Shell Namespace calls on virtual paths.
            if !allow_blocking {
                return None;
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

        // For other files (not unique icon types): check extension cache FIRST (no alloc needed).
        if !is_folder {
            if let Some(ext) = path.extension() {
                let ext_raw = ext.to_string_lossy().to_lowercase();
                // Map extensions that share the same shell icon (sys→dll etc.)
                let ext_str = crate::infrastructure::windows::icons::canonical_icon_ext(&ext_raw);
                let ext_key = format!("{}_{:?}", ext_str, size);

                if let Some(texture) = self.extension_cache.get(&ext_key) {
                    return Some(texture.clone());
                }
            }

            // Critical for extensionless files: async worker stores results in
            // icon_cache (path-based), not extension_cache. We must check it
            // before returning early in non-blocking mode.
            let cache_key = make_cache_key(path, size);
            if let Some(texture) = self.icon_cache.get(&cache_key) {
                return Some(texture.clone());
            }

            // Extension not cached yet. For non-blocking callers (render loop),
            // NEVER call get_file_type_icon synchronously — a single call can
            // take 100-500ms for a cold extension (COM/registry overhead).
            // Return None so file_slot triggers request_icon_load → async worker.
            // The worker loads the icon and populates extension_cache; subsequent
            // frames serve the icon instantly from cache.
            if !allow_blocking {
                return None;
            }
        }

        // Fallback: check icon_cache with full cache_key (lazy allocation only for misses).
        let cache_key = make_cache_key(path, size);
        if let Some(texture) = self.icon_cache.get(&cache_key) {
            return Some(texture.clone());
        }

        let icon_result = if is_folder {
            if !self.can_run_non_blocking_sync_icon_lookup(path, allow_blocking) {
                return None;
            }
            let lookup_start = std::time::Instant::now();
            let result = windows::get_file_type_icon(true, "", size);
            let elapsed = lookup_start.elapsed();
            if elapsed.as_millis() > 5 {
                log::warn!(
                    "[PERF-ICON] SLOW get_file_type_icon(folder) {}ms path={:?}",
                    elapsed.as_millis(),
                    path
                );
            }
            self.record_non_blocking_sync_icon_lookup(elapsed, allow_blocking);
            result
        } else if let Some(ext) = path.extension() {
            let ext_str = ext.to_string_lossy().to_lowercase();
            if !self.can_run_non_blocking_sync_icon_lookup(path, allow_blocking) {
                return None;
            }
            let lookup_start = std::time::Instant::now();
            let result = windows::get_file_type_icon(false, &ext_str, size);
            let elapsed = lookup_start.elapsed();
            if elapsed.as_millis() > 5 {
                log::warn!(
                    "[PERF-ICON] SLOW get_file_type_icon(ext={}) {}ms path={:?}",
                    ext_str,
                    elapsed.as_millis(),
                    path
                );
            }
            self.record_non_blocking_sync_icon_lookup(elapsed, allow_blocking);
            result
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
                if !self.can_run_non_blocking_sync_icon_lookup(path, allow_blocking) {
                    return None;
                }
                let lookup_start = std::time::Instant::now();
                let result = windows::get_file_type_icon(false, &ext, size);
                self.record_non_blocking_sync_icon_lookup(lookup_start.elapsed(), allow_blocking);
                result
            } else {
                if !self.can_run_non_blocking_sync_icon_lookup(path, allow_blocking) {
                    return None;
                }
                let lookup_start = std::time::Instant::now();
                let result = windows::get_file_type_icon(false, "", size);
                self.record_non_blocking_sync_icon_lookup(lookup_start.elapsed(), allow_blocking);
                result
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
                    let ext_raw = ext.to_string_lossy().to_lowercase();
                    let ext_str =
                        crate::infrastructure::windows::icons::canonical_icon_ext(&ext_raw);
                    let ext_key = format!("{}_{:?}", ext_str, size);
                    self.extension_cache.put(ext_key, texture.clone());
                }
            }

            self.icon_cache.put(cache_key, texture);
            return Some(cloned);
        }

        None
    }
}
