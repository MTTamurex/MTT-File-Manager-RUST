use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use lru::LruCache;

use crate::domain::file_entry::FileEntry;

const CACHE_CAPACITY: usize = 200; // Bounded to avoid high long-session RAM growth

struct CachedFolder {
    entries: Vec<FileEntry>,
    cached_at_ms: u64,
}

pub struct DirectoryCache {
    inner: Arc<Mutex<LruCache<PathBuf, CachedFolder>>>,
}

impl DirectoryCache {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(CACHE_CAPACITY).unwrap(),
            ))),
        }
    }

    /// Returns cached entries immediately if available.
    /// Cache validity is guaranteed by DriveWatcher: it monitors the entire drive
    /// and proactively invalidates entries on any filesystem change.
    pub fn get(&self, path: &PathBuf) -> Option<Vec<FileEntry>> {
        let mut cache = self.inner.lock().ok()?;
        if let Some(cached) = cache.get_mut(path) {
            return Some(cached.entries.clone());
        }
        None
    }

    /// Returns cached entries and the cache timestamp in Unix milliseconds.
    pub fn get_with_meta(&self, path: &PathBuf) -> Option<(Vec<FileEntry>, u64)> {
        let mut cache = self.inner.lock().ok()?;
        if let Some(cached) = cache.get_mut(path) {
            return Some((cached.entries.clone(), cached.cached_at_ms));
        }
        None
    }

    /// Store directory entries in cache.
    /// No fs::metadata() syscall — DriveWatcher handles invalidation.
    pub fn put(&self, path: PathBuf, entries: Vec<FileEntry>) {
        if let Ok(mut cache) = self.inner.lock() {
            let cached_at_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            cache.put(
                path,
                CachedFolder {
                    entries,
                    cached_at_ms,
                },
            );
        }
    }

    pub fn invalidate(&self, path: &PathBuf) {
        if let Ok(mut cache) = self.inner.lock() {
            let _ = cache.pop(path);
        }
    }

    pub fn invalidate_children(&self, parent: &PathBuf) {
        if let Ok(mut cache) = self.inner.lock() {
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
        if let Ok(mut cache) = self.inner.lock() {
            cache.clear();
        }
    }

    pub fn stats(&self) -> (usize, usize) {
        if let Ok(cache) = self.inner.lock() {
            let total_items: usize = cache.iter().map(|(_, v)| v.entries.len()).sum();
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
