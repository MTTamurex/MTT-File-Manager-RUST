//! Cache management for textures and icons
//! Follows .cursorrules: zero allocations in hot path, LRU eviction

use eframe::egui;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::path::PathBuf;

/// Texture cache configuration
pub struct TextureCacheConfig {
    pub max_size: usize,
    pub max_concurrent_loads: usize,
}

impl Default for TextureCacheConfig {
    fn default() -> Self {
        Self {
            max_size: 200,  // ~50-100MB VRAM
            max_concurrent_loads: 30,
        }
    }
}

/// Icon cache configuration
pub struct IconCacheConfig {
    pub max_size: usize,
}

impl Default for IconCacheConfig {
    fn default() -> Self {
        Self {
            max_size: 100,  // Icons are shared by extension
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
    
    config: TextureCacheConfig,
    icon_config: IconCacheConfig,
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
            
            config: TextureCacheConfig::default(),
            icon_config: IconCacheConfig::default(),
        }
    }
    
    /// Creates a cache manager with custom configuration
    pub fn with_config(config: TextureCacheConfig, icon_config: IconCacheConfig) -> Self {
        Self {
            texture_cache: LruCache::new(NonZeroUsize::new(config.max_size).unwrap()),
            icon_cache: LruCache::new(NonZeroUsize::new(icon_config.max_size).unwrap()),
            loading_set: std::collections::HashSet::new(),
            folder_icon_texture: None,
            computer_icon: None,
            drive_icon_cache: LruCache::new(NonZeroUsize::new(10).unwrap()),
            
            config,
            icon_config,
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
        // Note: folder_icon_texture and computer_icon are kept as they're singletons
    }
    
    /// Estimates VRAM usage in bytes
    pub fn estimate_vram_usage(&self) -> usize {
        let texture_usage: usize = self.texture_cache.iter()
            .map(|(_, tex)| {
                let size = tex.size();
                size[0] * size[1] * 4  // RGBA = 4 bytes per pixel
            })
            .sum();
        
        let icon_usage: usize = self.icon_cache.iter()
            .map(|(_, tex)| {
                let size = tex.size();
                size[0] * size[1] * 4
            })
            .sum();
        
        let drive_icon_usage: usize = self.drive_icon_cache.iter()
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
                
                self.computer_icon = Some(ctx.load_texture(
                    "computer_icon",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
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
