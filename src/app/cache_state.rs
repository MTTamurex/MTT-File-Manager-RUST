use crate::infrastructure::directory_cache::DirectoryCache;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::infrastructure::filesystem_cache::{FileSystemCache, FileSystemCacheConfig};
use crate::ui::cache::CacheManager;
use crate::ui::cache::FxHashSet;
use lru::LruCache;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Estado de gerenciamento de cache
pub struct CacheState {
    pub cache_manager: Arc<Mutex<CacheManager>>, // Wrap in Mutex for safe mutability
    pub disk_cache: Arc<ThumbnailDiskCache>,
    pub directory_cache: Arc<DirectoryCache>,
    pub filesystem_cache: Arc<FileSystemCache>, // NEW: High-performance filesystem cache
    pub directory_index: Option<Arc<crate::infrastructure::directory_index::DirectoryIndex>>,
    pub metadata_cache: LruCache<PathBuf, (u64, crate::infrastructure::windows::MediaMetadata)>,
    pub metadata_loading: FxHashSet<PathBuf>,
    pub last_metadata_refresh: Instant,
    pub last_metadata_path: Option<PathBuf>,
}

impl CacheState {
    pub fn new() -> Self {
        // Criar diretório de cache temporário
        let cache_dir = std::env::temp_dir().join("mtt-thumbnail-cache");
        
        // Configure filesystem cache for optimal performance
        let fs_cache_config = FileSystemCacheConfig {
            max_directories: 200,      // 200 folders in RAM
            max_age_seconds: 300,        // 5 minutes
            enable_revalidation: true,   // Background checks
            max_entries_per_dir: 20000,  // Max 20k files per folder
        };
        
        Self {
            cache_manager: Arc::new(Mutex::new(CacheManager::new())),
            disk_cache: Arc::new(ThumbnailDiskCache::new(cache_dir)),
            directory_cache: Arc::new(DirectoryCache::new()),
            filesystem_cache: Arc::new(FileSystemCache::new(fs_cache_config)),
            directory_index: None,
            metadata_cache: LruCache::new(std::num::NonZeroUsize::new(100).unwrap()),
            metadata_loading: FxHashSet::default(),
            last_metadata_refresh: Instant::now(),
            last_metadata_path: None,
        }
    }
    
    /// Limpa todos os caches
    pub fn clear_all(&mut self) {
        // Agora podemos acessar o CacheManager de forma segura
        if let Ok(mut cache_manager) = self.cache_manager.lock() {
            cache_manager.clear_all();
        }
        self.filesystem_cache.clear();
        self.metadata_cache.clear();
        self.metadata_loading.clear();
    }
}