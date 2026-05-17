use crate::infrastructure::app_state_db::AppStateDb;
use crate::infrastructure::diagnostic_logger::{diag_error, field_label};
use crate::infrastructure::directory_cache::DirectoryCache;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::ui::cache::CacheManager;
use crate::ui::cache::FxHashSet;
use lru::LruCache;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Cache management state
pub struct CacheState {
    pub cache_manager: Arc<Mutex<CacheManager>>,
    pub disk_cache: Arc<ThumbnailDiskCache>,
    pub app_state_db: Arc<AppStateDb>,
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
        let disk_cache = match ThumbnailDiskCache::new(cache_dir.clone()) {
            Ok(cache) => Arc::new(cache),
            Err(e) => {
                let _ = (cache_dir, e);
                log::error!("[Cache] Fatal: failed to initialize thumbnail disk cache state");
                diag_error(
                    "cache_state",
                    "thumbnail_disk_cache_init_failed",
                    &[field_label("scope", "temporary_cache")],
                );
                std::process::exit(1);
            }
        };

        let state_dir = std::env::temp_dir().join("mtt-state-cache");
        let app_state_db = match AppStateDb::new(state_dir.clone()) {
            Ok(db) => Arc::new(db),
            Err(e) => {
                let _ = (state_dir, e);
                log::error!("[State] Fatal: failed to initialize application state database");
                diag_error(
                    "cache_state",
                    "app_state_db_init_failed",
                    &[field_label("scope", "temporary_state_db")],
                );
                std::process::exit(1);
            }
        };

        Self {
            cache_manager: Arc::new(Mutex::new(CacheManager::new())),
            disk_cache,
            app_state_db,
            directory_cache: Arc::new(DirectoryCache::new()),
            directory_index: None,
            metadata_cache: LruCache::new(
                std::num::NonZeroUsize::new(100)
                    .expect("cache_state metadata cache size must be non-zero"),
            ),
            metadata_loading: FxHashSet::default(),
            last_metadata_refresh: Instant::now(),
            last_metadata_path: None,
        }
    }

    /// Clear all caches
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
