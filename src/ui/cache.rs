//! Cache management for textures and icons
//! Follows .cursorrules: zero allocations in hot path, LRU eviction

use eframe::egui;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::path::PathBuf;

use std::collections::HashSet;

/// Texture cache configuration
pub struct TextureCacheConfig {
    pub max_size: usize,
    pub max_concurrent_loads: usize,
}

impl Default for TextureCacheConfig {
    fn default() -> Self {
        Self {
            max_size: 200, // ~50-100MB VRAM
            max_concurrent_loads: 30,
        }
    }
}

/// Manages texture caches for thumbnails and icons
pub struct CacheManager {
    pub texture_cache: LruCache<PathBuf, egui::TextureHandle>,
    pub icon_cache: LruCache<String, egui::TextureHandle>,
    pub loading_set: std::collections::HashSet<PathBuf>,
    pub folder_icon_texture: Option<egui::TextureHandle>,
    pub computer_icon: Option<egui::TextureHandle>,
    pub drive_icon_cache: LruCache<String, egui::TextureHandle>,
    /// Cache for folder preview thumbnails (sandwich effect)
    pub folder_preview_cache: LruCache<PathBuf, egui::TextureHandle>,
    /// Set of folder paths currently being loaded
    pub folder_preview_loading: HashSet<PathBuf>,
    /// Set of paths that failed thumbnail extraction (LRU bounded to 1000)
    pub failed_thumbnails: LruCache<PathBuf, ()>,

    config: TextureCacheConfig,
}

impl CacheManager {
    /// Creates a new cache manager with default configuration
    pub fn new() -> Self {
        Self {
            texture_cache: LruCache::new(NonZeroUsize::new(100).unwrap()),
            icon_cache: LruCache::new(NonZeroUsize::new(100).unwrap()),
            loading_set: std::collections::HashSet::new(),
            folder_icon_texture: None,
            computer_icon: None,
            drive_icon_cache: LruCache::new(NonZeroUsize::new(10).unwrap()),
            folder_preview_cache: LruCache::new(NonZeroUsize::new(100).unwrap()),
            folder_preview_loading: HashSet::new(),
            failed_thumbnails: LruCache::new(NonZeroUsize::new(1000).unwrap()),

            config: TextureCacheConfig::default(),
        }
    }

    /// Creates a cache manager with custom configuration
    pub fn with_config(config: TextureCacheConfig) -> Self {
        Self {
            texture_cache: LruCache::new(NonZeroUsize::new(config.max_size).unwrap()),
            icon_cache: LruCache::new(NonZeroUsize::new(100).unwrap()),
            loading_set: std::collections::HashSet::new(),
            folder_icon_texture: None,
            computer_icon: None,
            drive_icon_cache: LruCache::new(NonZeroUsize::new(10).unwrap()),
            folder_preview_cache: LruCache::new(NonZeroUsize::new(100).unwrap()),
            folder_preview_loading: HashSet::new(),
            failed_thumbnails: LruCache::new(NonZeroUsize::new(1000).unwrap()),

            config,
        }
    }

    /// Checks if a thumbnail is in the cache
    pub fn has_thumbnail(&self, path: &PathBuf) -> bool {
        self.texture_cache.contains(path)
    }

    /// Gets a thumbnail from the cache
    pub fn get_thumbnail(&mut self, path: &PathBuf) -> Option<&egui::TextureHandle> {
        self.texture_cache.get(path)
    }

    /// Puts a thumbnail in the cache
    pub fn put_thumbnail(&mut self, path: PathBuf, texture: egui::TextureHandle) {
        self.texture_cache.put(path, texture);
    }

    /// Checks if a thumbnail is being loaded
    pub fn is_loading(&self, path: &PathBuf) -> bool {
        self.loading_set.contains(path)
    }

    /// Starts loading a thumbnail
    pub fn start_loading(&mut self, path: PathBuf) -> bool {
        if self.loading_set.len() < self.config.max_concurrent_loads {
            self.loading_set.insert(path);
            true
        } else {
            false
        }
    }

    /// Finishes loading a thumbnail
    pub fn finish_loading(&mut self, path: &PathBuf) {
        self.loading_set.remove(path);
    }

    /// Clears all caches
    pub fn clear_all(&mut self) {
        self.texture_cache.clear();
        self.icon_cache.clear();
        self.loading_set.clear();
        self.drive_icon_cache.clear();
        self.folder_preview_cache.clear();
        self.folder_preview_loading.clear();
        self.failed_thumbnails.clear();
        // Note: folder_icon_texture and computer_icon are kept as they're singletons
    }

    /// Marks a path as having failed thumbnail extraction
    pub fn mark_as_failed(&mut self, path: PathBuf) {
        self.failed_thumbnails.put(path, ());
    }

    /// Checks if a path has previously failed thumbnail extraction
    pub fn is_failed(&self, path: &PathBuf) -> bool {
        self.failed_thumbnails.contains(path)
    }

    /// Clears the failure status for all paths
    pub fn clear_failed(&mut self) {
        self.failed_thumbnails.clear();
    }

    // ========== Folder Preview Methods (Native Windows Shell) ==========

    /// Gets folder preview from cache
    pub fn get_folder_preview(&mut self, path: &PathBuf) -> Option<&egui::TextureHandle> {
        self.folder_preview_cache.get(path)
    }

    /// Checks if folder preview is in cache
    pub fn has_folder_preview(&self, path: &PathBuf) -> bool {
        self.folder_preview_cache.contains(path)
    }

    /// Stores folder preview in cache
    pub fn put_folder_preview(&mut self, path: PathBuf, texture: egui::TextureHandle) {
        self.folder_preview_cache.put(path, texture);
    }

    /// Checks if folder preview is currently being loaded
    pub fn is_folder_preview_loading(&self, path: &PathBuf) -> bool {
        self.folder_preview_loading.contains(path)
    }

    /// Starts loading a folder preview (returns false if too many loads in progress)
    pub fn start_folder_preview_loading(&mut self, path: PathBuf) -> bool {
        if self.folder_preview_loading.len() < 30 {
            self.folder_preview_loading.insert(path);
            true
        } else {
            false
        }
    }

    /// Finishes loading a folder preview
    pub fn finish_folder_preview_loading(&mut self, path: &PathBuf) {
        self.folder_preview_loading.remove(path);
    }

    /// Invalidates a folder preview (removes from cache and loading set)
    /// Called when folder contents change to trigger reload
    pub fn invalidate_folder_preview(&mut self, path: &PathBuf) {
        self.folder_preview_cache.pop(path);
        self.folder_preview_loading.remove(path);
    }
    /// Estimates VRAM usage in bytes
    pub fn estimate_vram_usage(&self) -> usize {
        let texture_usage: usize = self
            .texture_cache
            .iter()
            .map(|(_, tex)| {
                let size = tex.size();
                size[0] * size[1] * 4 // RGBA = 4 bytes per pixel
            })
            .sum();

        let icon_usage: usize = self
            .icon_cache
            .iter()
            .map(|(_, tex)| {
                let size = tex.size();
                size[0] * size[1] * 4
            })
            .sum();

        let drive_icon_usage: usize = self
            .drive_icon_cache
            .iter()
            .map(|(_, tex)| {
                let size = tex.size();
                size[0] * size[1] * 4
            })
            .sum();

        texture_usage + icon_usage + drive_icon_usage
    }

    /// Gets or creates a drive icon
    pub fn get_drive_icon(
        &mut self,
        ctx: &egui::Context,
        disk_path: &str,
        extract_fn: impl Fn(&str) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>>,
    ) -> Option<egui::TextureHandle> {
        if let Some(texture) = self.drive_icon_cache.get(disk_path) {
            return Some(texture.clone());
        }

        match extract_fn(disk_path) {
            Ok((rgba_data, width, height)) => {
                let texture = ctx.load_texture(
                    format!("drive_{}", disk_path),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba_data,
                    ),
                    egui::TextureOptions::LINEAR,
                );

                let cloned = texture.clone();
                self.drive_icon_cache.put(disk_path.to_string(), texture);
                Some(cloned)
            }
            Err(_) => None,
        }
    }

    /// Gets or creates a file icon
    pub fn get_file_icon(
        &mut self,
        ctx: &egui::Context,
        path: &PathBuf,
        extract_fn: impl Fn(&PathBuf) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>>,
        extension: &str,
    ) -> Option<egui::TextureHandle> {
        // Decide cache key: path completo para executáveis, extensão para demais
        let cache_key = if matches!(extension, "exe" | "lnk" | "ico") {
            // Cache por path completo - cada executável tem ícone único
            path.to_string_lossy().to_string()
        } else {
            // Cache por extensão - todos .txt compartilham ícone
            format!(".{}", extension)
        };

        // Cache hit? Clone do handle (barato)
        if let Some(texture) = self.icon_cache.get(&cache_key) {
            return Some(texture.clone());
        }

        // Cache miss -> carrega ícone
        match extract_fn(path) {
            Ok((rgba_data, width, height)) => {
                let texture = ctx.load_texture(
                    format!("icon_{}", cache_key),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba_data,
                    ),
                    egui::TextureOptions::LINEAR,
                );

                let cloned = texture.clone();
                self.icon_cache.put(cache_key, texture);
                Some(cloned)
            }
            Err(_) => None,
        }
    }

    /// Ensures folder icon is loaded
    pub fn ensure_folder_icon(
        &mut self,
        ctx: &egui::Context,
        extract_fn: impl Fn() -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>>,
    ) {
        if self.folder_icon_texture.is_some() {
            return;
        }

        match extract_fn() {
            Ok((rgba_data, width, height)) => {
                let texture = ctx.load_texture(
                    "folder_icon",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba_data,
                    ),
                    egui::TextureOptions::LINEAR,
                );
                self.folder_icon_texture = Some(texture);
            }
            Err(_) => {
                // Fallback: mantém emoji
            }
        }
    }

    /// Ensures computer icon is loaded
    pub fn ensure_computer_icon(
        &mut self,
        ctx: &egui::Context,
        extract_fn: impl Fn() -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>>,
    ) {
        if self.computer_icon.is_some() {
            return;
        }

        match extract_fn() {
            Ok((data, width, height)) => {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    &data,
                );

                self.computer_icon =
                    Some(ctx.load_texture("computer_icon", image, egui::TextureOptions::LINEAR));
            }
            Err(_) => {
                // Fallback
            }
        }
    }
}

impl Default for CacheManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_manager_creation() {
        let cache = CacheManager::new();
        assert_eq!(cache.texture_cache.len(), 0);
        assert_eq!(cache.icon_cache.len(), 0);
        assert!(cache.loading_set.is_empty());
    }

    #[test]
    fn test_loading_management() {
        let mut cache = CacheManager::new();
        let path = PathBuf::from("test.txt");

        assert!(!cache.is_loading(&path));
        assert!(cache.start_loading(path.clone()));
        assert!(cache.is_loading(&path));

        cache.finish_loading(&path);
        assert!(!cache.is_loading(&path));
    }

    #[test]
    fn test_vram_estimation() {
        let cache = CacheManager::new();
        let usage = cache.estimate_vram_usage();
        assert_eq!(usage, 0); // Empty cache
    }

    #[test]
    fn test_cache_clear() {
        let mut cache = CacheManager::new();
        cache.clear_all();
        assert_eq!(cache.texture_cache.len(), 0);
        assert_eq!(cache.icon_cache.len(), 0);
        assert!(cache.loading_set.is_empty());
    }
}
