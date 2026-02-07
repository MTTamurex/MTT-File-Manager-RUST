use crate::infrastructure::directory_cache::DirectoryCache;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::ui::cache::CacheManager;
use crate::ui::cache::FxHashSet;
use lru::LruCache;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Estado de gerenciamento de cache
pub struct CacheState {
    pub cache_manager: Arc<Mutex<CacheManager>>,
    pub disk_cache: Arc<ThumbnailDiskCache>,
    pub directory_cache: Arc<DirectoryCache>,
    pub directory_index: Option<Arc<crate::infrastructure::directory_index::DirectoryIndex>>,
    pub metadata_cache: LruCache<PathBuf, (u64, crate::infrastructure::windows::MediaMetadata)>,
    pub metadata_loading: FxHashSet<PathBuf>,
    pub last_metadata_refresh: Instant,
    pub last_metadata_path: Option<PathBuf>,
}

impl CacheState {
    pub fn new() -> Self {
        let cache_dir = std::env::temp_dir().join("mtt-thumbnail-cache");

        Self {
            cache_manager: Arc::new(Mutex::new(CacheManager::new())),
            disk_cache: Arc::new(ThumbnailDiskCache::new(cache_dir)),
            directory_cache: Arc::new(DirectoryCache::new()),
            directory_index: None,
            metadata_cache: LruCache::new(std::num::NonZeroUsize::new(100).unwrap()),
            metadata_loading: FxHashSet::default(),
            last_metadata_refresh: Instant::now(),
            last_metadata_path: None,
        }
    }

    /// Limpa todos os caches
    pub fn clear_all(&mut self) {
        if let Ok(mut cache_manager) = self.cache_manager.lock() {
            cache_manager.clear_all();
        }
        self.metadata_cache.clear();
        self.metadata_loading.clear();
    }
}

impl Default for CacheState {
    fn default() -> Self {
        Self::new()
    }
}
