//! Enhanced FileSystemCache with cache-first strategy and background revalidation
//!
//! This module provides:
//! - Cache-first navigation (0ms latency for cached folders)
//! - Background revalidation to detect changes
//! - Thread-safe concurrent access
//! - LRU eviction with configurable limits

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use dashmap::DashMap;
use lru::LruCache;

use crate::domain::file_entry::FileEntry;

/// Configuration for FileSystemCache
#[derive(Debug, Clone)]
pub struct FileSystemCacheConfig {
    /// Maximum number of cached directories
    pub max_directories: usize,
    /// Maximum age for cache entries (seconds)
    pub max_age_seconds: u64,
    /// Enable background revalidation
    pub enable_revalidation: bool,
    /// Maximum size for individual directory entries
    pub max_entries_per_dir: usize,
}

impl Default for FileSystemCacheConfig {
    fn default() -> Self {
        Self {
            max_directories: 100,      // 100 folders in RAM
            max_age_seconds: 300,      // 5 minutes
            enable_revalidation: true,  // Background checks
            max_entries_per_dir: 10000, // Max 10k files per folder
        }
    }
}

/// Cached directory entry with metadata
#[derive(Debug, Clone)]
pub struct CachedDirectory {
    /// File entries in this directory
    pub entries: Vec<FileEntry>,
    /// When this cache entry was created
    pub cached_at: Instant,
    /// Directory modification time when cached
    pub dir_mtime: Option<SystemTime>,
    /// Number of items (for stats)
    pub item_count: usize,
    /// Cache hit counter (for LRU)
    pub hit_count: u64,
}

impl CachedDirectory {
    pub fn new(entries: Vec<FileEntry>, dir_mtime: Option<SystemTime>) -> Self {
        let item_count = entries.len();
        Self {
            entries,
            cached_at: Instant::now(),
            dir_mtime,
            item_count,
            hit_count: 0,
        }
    }

    /// Check if this cache entry is still valid based on age
    pub fn is_valid(&self, max_age: Duration) -> bool {
        self.cached_at.elapsed() < max_age
    }

    /// Increment hit counter and return entries
    pub fn get_entries(&mut self) -> Vec<FileEntry> {
        self.hit_count += 1;
        self.entries.clone()
    }
}

/// High-performance filesystem cache with cache-first strategy
///
/// Uses DashMap for concurrent access and LRU for memory management.
/// Provides instant navigation for previously visited folders.
pub struct FileSystemCache {
    /// Main cache storage - concurrent hash map
    cache: Arc<DashMap<String, CachedDirectory>>,
    /// LRU tracker for eviction (stores keys only)
    lru: Arc<Mutex<LruCache<String, ()>>>,
    /// Configuration
    config: FileSystemCacheConfig,
}

impl FileSystemCache {
    /// Create a new filesystem cache with given configuration
    pub fn new(config: FileSystemCacheConfig) -> Self {
        let cache = Arc::new(DashMap::new());
        let lru = Arc::new(Mutex::new(LruCache::new(
            std::num::NonZeroUsize::new(config.max_directories).unwrap()
        )));

        Self { cache, lru, config }
    }

    /// Try to get cached directory entries (cache-first strategy)
    ///
    /// Returns None if not cached or expired.
    /// Increments hit counter if found.
    pub fn get(&self, path: &Path) -> Option<Vec<FileEntry>> {
        let path_str = path.to_string_lossy().to_string();
        
        // Fast path: try to get from cache
        if let Some(mut entry) = self.cache.get_mut(&path_str) {
            // Check if still valid
            if entry.is_valid(Duration::from_secs(self.config.max_age_seconds)) {
                // Update LRU (mark as recently used)
                if let Ok(mut lru) = self.lru.lock() {
                    let _ = lru.get(&path_str);
                }
                return Some(entry.get_entries());
            }
        }
        
        None
    }

    /// Store directory entries in cache
    ///
    /// Automatically handles LRU eviction if cache is full.
    pub fn put(&self, path: &Path, entries: Vec<FileEntry>, dir_mtime: Option<SystemTime>) {
        // Limit entries per directory to prevent memory issues
        let entries = if entries.len() > self.config.max_entries_per_dir {
            entries[..self.config.max_entries_per_dir].to_vec()
        } else {
            entries
        };

        let path_str = path.to_string_lossy().to_string();
        let cached_dir = CachedDirectory::new(entries, dir_mtime);

        // Check if we need to evict before adding
        if self.cache.len() >= self.config.max_directories {
            self.evict_oldest();
        }

        // Add to cache
        self.cache.insert(path_str.clone(), cached_dir);

        // Update LRU
        if let Ok(mut lru) = self.lru.lock() {
            let _ = lru.put(path_str, ());
        }
    }

    /// Check if a path is cached (without updating hit count)
    pub fn contains(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy().to_string();
        self.cache.contains_key(&path_str)
    }

    /// Get cache statistics
    pub fn stats(&self) -> (usize, usize) {
        let total_entries: usize = self.cache.iter()
            .map(|entry| entry.value().item_count)
            .sum();
        (self.cache.len(), total_entries)
    }

    /// Clear all cached entries
    pub fn clear(&self) {
        self.cache.clear();
        if let Ok(mut lru) = self.lru.lock() {
            lru.clear();
        }
    }

    /// Invalidate a specific path
    pub fn invalidate(&self, path: &Path) {
        let path_str = path.to_string_lossy().to_string();
        self.cache.remove(&path_str);
        if let Ok(mut lru) = self.lru.lock() {
            lru.pop(&path_str);
        }
    }

    /// Invalidate all child directories of a parent path
    pub fn invalidate_children(&self, parent: &Path) {
        let parent_str = parent.to_string_lossy().to_string();
        let keys_to_remove: Vec<String> = self.cache.iter()
            .filter(|entry| entry.key().starts_with(&parent_str))
            .map(|entry| entry.key().clone())
            .collect();

        for key in keys_to_remove {
            self.cache.remove(&key);
            if let Ok(mut lru) = self.lru.lock() {
                lru.pop(&key);
            }
        }
    }

    /// Evict oldest entry based on LRU
    fn evict_oldest(&self) {
        if let Ok(mut lru) = self.lru.lock() {
            if let Some((oldest_key, _)) = lru.pop_lru() {
                self.cache.remove(&oldest_key);
            }
        }
    }

    /// Get directory modification time for a path
    pub fn get_dir_mtime(&self, path: &Path) -> Option<SystemTime> {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
    }

    /// Check if cache entry needs revalidation
    pub fn needs_revalidation(&self, path: &Path) -> bool {
        if !self.config.enable_revalidation {
            return false;
        }

        let path_str = path.to_string_lossy().to_string();
        if let Some(entry) = self.cache.get(&path_str) {
            // Check if entry is older than half the max age (for proactive revalidation)
            let half_max_age = Duration::from_secs(self.config.max_age_seconds / 2);
            if entry.cached_at.elapsed() > half_max_age {
                return true;
            }

            // Check if directory modification time changed
            if let Some(cached_mtime) = entry.dir_mtime {
                if let Some(current_mtime) = self.get_dir_mtime(path) {
                    return cached_mtime != current_mtime;
                }
            }
        }
        false
    }
}

impl Default for FileSystemCache {
    fn default() -> Self {
        Self::new(FileSystemCacheConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_cache_basic_operations() {
        let cache = FileSystemCache::default();
        let path = Path::new("C:\\test");
        
        // Initially empty
        assert!(cache.get(path).is_none());
        assert!(!cache.contains(path));
        
        // Add entries
        let entries = vec![
            FileEntry {
                path: PathBuf::from("C:\\test\\file1.txt"),
                name: "file1.txt".to_string(),
                is_dir: false,
                size: 100,
                modified: 1234567890,
                folder_cover: None,
                drive_info: None,
                sync_status: crate::domain::file_entry::SyncStatus::None,
                deletion_date: None,
                recycle_original_path: None,
            }
        ];
        
        cache.put(path, entries.clone(), None);
        
        // Should be cached
        assert!(cache.contains(path));
        let cached = cache.get(path).unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].name, "file1.txt");
        
        // Stats
        let (dirs, items) = cache.stats();
        assert_eq!(dirs, 1);
        assert_eq!(items, 1);
    }

    #[test]
    fn test_cache_invalidation() {
        // Test simplified - just ensure no panic
        let cache = FileSystemCache::default();
        let path = Path::new("C:/test");
        
        // Add entry
        cache.put(path, vec![], None);
        assert!(cache.contains(path));
        
        // Invalidate
        cache.invalidate(path);
        assert!(!cache.contains(path));
    }
}