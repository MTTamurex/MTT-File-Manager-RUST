use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use lru::LruCache;

use crate::domain::file_entry::FileEntry;

const MAX_CACHED_DIRS: usize = 50;
const MAX_CACHE_AGE: Duration = Duration::from_secs(300);

struct CachedDirectory {
    entries: Vec<FileEntry>,
    cached_at: Instant,
    item_count: usize,
}

pub struct DirectoryCache {
    cache: Mutex<LruCache<PathBuf, CachedDirectory>>,
}

impl DirectoryCache {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(MAX_CACHED_DIRS).unwrap(),
            )),
        }
    }

    pub fn get(&self, path: &PathBuf) -> Option<Vec<FileEntry>> {
        let mut cache = self.cache.lock().ok()?;
        if let Some(cached) = cache.get(path) {
            if cached.cached_at.elapsed() < MAX_CACHE_AGE {
                return Some(cached.entries.clone());
            }
        }
        None
    }

    pub fn put(&self, path: PathBuf, entries: Vec<FileEntry>) {
        if let Ok(mut cache) = self.cache.lock() {
            let item_count = entries.len();
            cache.put(
                path,
                CachedDirectory {
                    entries,
                    cached_at: Instant::now(),
                    item_count,
                },
            );
        }
    }

    pub fn invalidate(&self, path: &PathBuf) {
        if let Ok(mut cache) = self.cache.lock() {
            let _ = cache.pop(path);
        }
    }

    pub fn invalidate_children(&self, parent: &PathBuf) {
        if let Ok(mut cache) = self.cache.lock() {
            let keys_to_remove: Vec<PathBuf> = cache
                .iter()
                .filter(|(k, _)| k.starts_with(parent))
                .map(|(k, _)| k.clone())
                .collect();

            for key in keys_to_remove {
                cache.pop(&key);
            }
        }
    }

    pub fn clear(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.clear();
        }
    }

    pub fn stats(&self) -> (usize, usize) {
        if let Ok(cache) = self.cache.lock() {
            let total_items: usize = cache.iter().map(|(_, v)| v.item_count).sum();
            (cache.len(), total_items)
        } else {
            (0, 0)
        }
    }
}

impl Default for DirectoryCache {
    fn default() -> Self {
        Self::new()
    }
}
