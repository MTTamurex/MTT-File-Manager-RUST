//! Persistent disk cache for extension-based file icons.
//!
//! Stores raw RGBA pixel data per extension so that subsequent app launches
//! can populate the `extension_cache` instantly without calling `SHGetFileInfoW`.
//!
//! File format per extension: `{ext}.rgba`
//!   [width: u32 LE][height: u32 LE][rgba_pixels...]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// On-disk cache for extension → RGBA icon data.
pub struct IconDiskCache {
    dir: PathBuf,
}

impl IconDiskCache {
    /// Create (or open) the icon disk cache directory.
    pub fn new(app_data_dir: &Path) -> Self {
        let dir = app_data_dir.join("extension_icons");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("[IconDiskCache] Failed to create dir {:?}: {}", dir, e);
        }
        Self { dir }
    }

    /// Load ALL cached extension icons from disk.
    /// Returns `HashMap<extension_lowercase, (rgba_pixels, width, height)>`.
    /// Typically completes in <5ms for ~100 extensions (files are tiny, OS-cached).
    pub fn load_all(&self) -> HashMap<String, (Vec<u8>, u32, u32)> {
        let mut map = HashMap::with_capacity(128);
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return map,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rgba") {
                continue;
            }
            let ext = match path.file_stem().and_then(|s| s.to_str()) {
                Some(e) => e.to_lowercase(),
                None => continue,
            };

            // If this extension maps to a different canonical form (e.g. sys→dll),
            // the cached icon is stale (wrong icon from a pre-mapping session).
            // Delete the file and skip — the worker will re-extract under the
            // canonical key on the next run.
            let canonical = crate::infrastructure::windows::icons::canonical_icon_ext(&ext);
            if canonical != ext {
                log::info!(
                    "[IconDiskCache] Removing stale mapped icon {:?} (canonical={})",
                    path, canonical,
                );
                let _ = std::fs::remove_file(&path);
                continue;
            }
            if crate::infrastructure::windows::icons::requires_real_file_for_shared_icon(&ext) {
                log::info!(
                    "[IconDiskCache] Removing path-seeded icon cache {:?} (must be rebuilt from a real file)",
                    path,
                );
                let _ = std::fs::remove_file(&path);
                continue;
            }
            let data = match std::fs::read(&path) {
                Ok(d) => d,
                Err(_) => {
                    // Corrupted file — remove it.
                    let _ = std::fs::remove_file(&path);
                    continue;
                }
            };
            if data.len() < 8 {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            let width = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            let height = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
            let pixels = data[8..].to_vec();
            let expected = (width as usize) * (height as usize) * 4;
            if pixels.len() != expected || width == 0 || height == 0 {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            map.insert(ext, (pixels, width, height));
        }
        if !map.is_empty() {
            log::info!(
                "[IconDiskCache] Loaded {} cached extension icons from disk",
                map.len()
            );
        }
        map
    }

    /// Save an extension's icon data to disk.
    /// Called from worker threads after extracting a new extension icon.
    pub fn save(&self, ext: &str, pixels: &[u8], width: u32, height: u32) {
        if ext.is_empty() || pixels.is_empty() || width == 0 || height == 0 {
            return;
        }
        // Always save under the canonical extension so mapped types (sys→dll)
        // share a single cache file.
        let canonical = crate::infrastructure::windows::icons::canonical_icon_ext(ext);
        if crate::infrastructure::windows::icons::requires_real_file_for_shared_icon(canonical) {
            return;
        }
        let path = self.dir.join(format!("{}.rgba", canonical.to_lowercase()));
        // Don't overwrite if already exists (another worker may have written it).
        if path.exists() {
            return;
        }
        let mut data = Vec::with_capacity(8 + pixels.len());
        data.extend_from_slice(&width.to_le_bytes());
        data.extend_from_slice(&height.to_le_bytes());
        data.extend_from_slice(pixels);
        if let Err(e) = std::fs::write(&path, &data) {
            log::warn!("[IconDiskCache] Failed to write {:?}: {}", path, e);
        }
    }
}
