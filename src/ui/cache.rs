//! Cache management for textures and icons
//! Follows .cursorrules: zero allocations in hot path, LRU eviction

use eframe::egui;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::path::PathBuf;

// PERFORMANCE: FxHashSet uses a faster hash function than std::collections::HashSet.
// This is especially beneficial for PathBuf keys which have expensive default hashing.
// FxHash is ~2-4x faster for string-like keys.
// Re-exported for use in other modules.
pub use rustc_hash::FxHashSet;

const DEFAULT_TEXTURE_CACHE_ITEMS: usize = 220;
const DEFAULT_FOLDER_PREVIEW_CACHE_ITEMS: usize = 80;
const DEFAULT_RGBA_CACHE_ITEMS: usize = 240;
const DEFAULT_MAX_CONCURRENT_LOADS: usize = 80;
const DEFAULT_RGBA_BUDGET_BYTES: usize = 128 * 1024 * 1024;

/// Texture cache configuration
pub struct TextureCacheConfig {
    pub max_size: usize,
    pub max_concurrent_loads: usize,
}

impl Default for TextureCacheConfig {
    fn default() -> Self {
        Self {
            max_size: DEFAULT_TEXTURE_CACHE_ITEMS,
            max_concurrent_loads: DEFAULT_MAX_CONCURRENT_LOADS,
        }
    }
}

/// Manages texture caches for thumbnails and icons
pub struct CacheManager {
    pub texture_cache: LruCache<PathBuf, egui::TextureHandle>,
    pub icon_cache: LruCache<String, egui::TextureHandle>,
    pub loading_set: FxHashSet<PathBuf>,
    pub folder_icon_texture: Option<egui::TextureHandle>,
    pub computer_icon: Option<egui::TextureHandle>,
    pub drive_icon_cache: LruCache<String, egui::TextureHandle>,
    /// Cache for folder preview thumbnails (sandwich effect)
    pub folder_preview_cache: LruCache<PathBuf, egui::TextureHandle>,
    /// Set of folder paths currently being loaded
    pub folder_preview_loading: FxHashSet<PathBuf>,
    /// Set of paths that failed thumbnail extraction (LRU bounded to 1000)
    pub failed_thumbnails: LruCache<PathBuf, ()>,
    /// Set of paths received from worker but waiting for GPU upload
    pub pending_upload_set: FxHashSet<PathBuf>,
    /// PERFORMANCE: RAM cache for decoded RGBA data (Layer 2 - larger than VRAM cache)
    /// When a texture is evicted from VRAM, the RGBA data remains here for fast re-upload
    /// without needing disk I/O. This is critical for HDD performance during video playback.
    pub rgba_data_cache: LruCache<PathBuf, (Vec<u8>, u32, u32)>,
    rgba_data_bytes: usize,
    max_rgba_data_bytes: usize,

    config: TextureCacheConfig,
}

impl CacheManager {
    /// Creates a new cache manager with default configuration
    pub fn new() -> Self {
        Self {
            // Bounded default keeps enough history for smooth scrolling without runaway RAM.
            texture_cache: LruCache::new(NonZeroUsize::new(DEFAULT_TEXTURE_CACHE_ITEMS).unwrap()),
            icon_cache: LruCache::new(NonZeroUsize::new(100).unwrap()),
            loading_set: FxHashSet::default(),
            folder_icon_texture: None,
            computer_icon: None,
            drive_icon_cache: LruCache::new(NonZeroUsize::new(10).unwrap()),
            folder_preview_cache: LruCache::new(
                NonZeroUsize::new(DEFAULT_FOLDER_PREVIEW_CACHE_ITEMS).unwrap(),
            ),
            folder_preview_loading: FxHashSet::default(),
            failed_thumbnails: LruCache::new(NonZeroUsize::new(1000).unwrap()),
            pending_upload_set: FxHashSet::default(),
            rgba_data_cache: LruCache::new(NonZeroUsize::new(DEFAULT_RGBA_CACHE_ITEMS).unwrap()),
            rgba_data_bytes: 0,
            max_rgba_data_bytes: DEFAULT_RGBA_BUDGET_BYTES,

            config: TextureCacheConfig::default(),
        }
    }

    /// Creates a cache manager with custom configuration
    pub fn with_config(config: TextureCacheConfig) -> Self {
        let rgba_cache_items = (config.max_size * 6 / 5).max(DEFAULT_RGBA_CACHE_ITEMS);
        let rgba_budget_bytes =
            (config.max_size * 1024 * 1024 / 2).clamp(DEFAULT_RGBA_BUDGET_BYTES, 256 * 1024 * 1024);

        Self {
            texture_cache: LruCache::new(NonZeroUsize::new(config.max_size).unwrap()),
            icon_cache: LruCache::new(NonZeroUsize::new(100).unwrap()),
            loading_set: FxHashSet::default(),
            folder_icon_texture: None,
            computer_icon: None,
            drive_icon_cache: LruCache::new(NonZeroUsize::new(10).unwrap()),
            folder_preview_cache: LruCache::new(
                NonZeroUsize::new(DEFAULT_FOLDER_PREVIEW_CACHE_ITEMS).unwrap(),
            ),
            folder_preview_loading: FxHashSet::default(),
            failed_thumbnails: LruCache::new(NonZeroUsize::new(1000).unwrap()),
            pending_upload_set: FxHashSet::default(),
            rgba_data_cache: LruCache::new(NonZeroUsize::new(rgba_cache_items).unwrap()),
            rgba_data_bytes: 0,
            max_rgba_data_bytes: rgba_budget_bytes,

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

    /// Checks if a thumbnail is waiting for upload
    pub fn is_pending_upload(&self, path: &PathBuf) -> bool {
        self.pending_upload_set.contains(path)
    }

    /// Marks a thumbnail as waiting for upload
    pub fn start_pending_upload(&mut self, path: PathBuf) {
        self.pending_upload_set.insert(path);
    }

    /// Removes a thumbnail from pending upload status
    pub fn finish_pending_upload(&mut self, path: &PathBuf) {
        self.pending_upload_set.remove(path);
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
        self.pending_upload_set.clear();
        self.rgba_data_cache.clear();
        self.rgba_data_bytes = 0;
        // Note: folder_icon_texture and computer_icon are kept as they're singletons
    }

    // ========== RAM Cache Methods (Layer 2 - RGBA Data) ==========

    /// Checks if RGBA data is in the RAM cache
    pub fn has_rgba_data(&self, path: &PathBuf) -> bool {
        self.rgba_data_cache.contains(path)
    }

    /// Gets RGBA data from the RAM cache
    pub fn get_rgba_data(&mut self, path: &PathBuf) -> Option<&(Vec<u8>, u32, u32)> {
        self.rgba_data_cache.get(path)
    }

    /// Stores RGBA data in the RAM cache
    pub fn put_rgba_data(&mut self, path: PathBuf, data: Vec<u8>, width: u32, height: u32) {
        let new_bytes = data.len();

        if let Some((old_data, _, _)) = self.rgba_data_cache.pop(&path) {
            self.rgba_data_bytes = self.rgba_data_bytes.saturating_sub(old_data.len());
        }

        self.rgba_data_cache.put(path, (data, width, height));
        self.rgba_data_bytes = self.rgba_data_bytes.saturating_add(new_bytes);
        self.enforce_rgba_budget(self.max_rgba_data_bytes);
    }

    /// Removes RGBA data for a specific path and updates memory accounting.
    pub fn pop_rgba_data(&mut self, path: &PathBuf) -> Option<(Vec<u8>, u32, u32)> {
        if let Some((data, width, height)) = self.rgba_data_cache.pop(path) {
            self.rgba_data_bytes = self.rgba_data_bytes.saturating_sub(data.len());
            Some((data, width, height))
        } else {
            None
        }
    }

    /// Trims thumbnail-related caches to target sizes.
    /// Returns `(textures_removed, rgba_removed, folder_previews_removed)`.
    pub fn trim_thumbnail_caches(
        &mut self,
        target_texture_items: usize,
        target_rgba_bytes: usize,
        target_folder_preview_items: usize,
    ) -> (usize, usize, usize) {
        let mut textures_removed = 0;
        let mut rgba_removed = 0;
        let mut folder_previews_removed = 0;

        while self.texture_cache.len() > target_texture_items {
            if let Some((path, _)) = self.texture_cache.pop_lru() {
                self.pending_upload_set.remove(&path);
                textures_removed += 1;
            } else {
                break;
            }
        }

        while self.folder_preview_cache.len() > target_folder_preview_items {
            if self.folder_preview_cache.pop_lru().is_some() {
                folder_previews_removed += 1;
            } else {
                break;
            }
        }

        while self.rgba_data_bytes > target_rgba_bytes {
            if let Some((_, (data, _, _))) = self.rgba_data_cache.pop_lru() {
                self.rgba_data_bytes = self.rgba_data_bytes.saturating_sub(data.len());
                rgba_removed += 1;
            } else {
                self.rgba_data_bytes = 0;
                break;
            }
        }

        (textures_removed, rgba_removed, folder_previews_removed)
    }

    fn enforce_rgba_budget(&mut self, budget_bytes: usize) {
        while self.rgba_data_bytes > budget_bytes {
            if let Some((_, (data, _, _))) = self.rgba_data_cache.pop_lru() {
                self.rgba_data_bytes = self.rgba_data_bytes.saturating_sub(data.len());
            } else {
                self.rgba_data_bytes = 0;
                break;
            }
        }
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

    /// Estimates RAM usage by the RGBA data cache in bytes
    pub fn estimate_ram_cache_usage(&self) -> usize {
        self.rgba_data_bytes
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

    #[test]
    fn test_rgba_accounting_updates_on_insert_and_remove() {
        let mut cache = CacheManager::new();
        let path = PathBuf::from("img.webp");

        cache.put_rgba_data(path.clone(), vec![1; 16], 2, 2);
        assert_eq!(cache.estimate_ram_cache_usage(), 16);

        cache.put_rgba_data(path.clone(), vec![2; 8], 2, 1);
        assert_eq!(cache.estimate_ram_cache_usage(), 8);

        let _ = cache.pop_rgba_data(&path);
        assert_eq!(cache.estimate_ram_cache_usage(), 0);
    }
}
