use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use lru::LruCache;

use crate::domain::file_entry::FileEntry;

const CACHE_CAPACITY: usize = 200; // Bounded to avoid high long-session RAM growth

struct CachedFolder {
    entries: Arc<Vec<FileEntry>>,
    cached_at_ms: u64,
}

pub struct DirectoryCache {
    inner: Arc<Mutex<LruCache<PathBuf, CachedFolder>>>,
}

impl DirectoryCache {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(CACHE_CAPACITY).expect("CACHE_CAPACITY must be non-zero"),
            ))),
        }
    }

    /// Returns cached entries immediately if available.
    /// Cache is invalidated by: DriveWatcher (when enabled), per-folder
    /// notify-watcher, consistency probe, and mtime validation in fast_paths.
    ///
    /// NOTE: `folder_cover` is stripped at `put()` time — it is resolved
    /// separately via the cover pipeline (SQLite + existence check + cover
    /// worker) to avoid returning stale covers from a previous visit.
    pub fn get(&self, path: &PathBuf) -> Option<Arc<Vec<FileEntry>>> {
        let mut cache = self.inner.lock().unwrap_or_else(|e| {
            log::warn!("[DIR-CACHE] Mutex poisoned in get(), recovering");
            e.into_inner()
        });
        cache.get_mut(path).map(|cached| Arc::clone(&cached.entries))
    }

    /// Returns cached entries and the cache timestamp in Unix milliseconds.
    pub fn get_with_meta(&self, path: &PathBuf) -> Option<(Arc<Vec<FileEntry>>, u64)> {
        let mut cache = self.inner.lock().unwrap_or_else(|e| {
            log::warn!("[DIR-CACHE] Mutex poisoned in get_with_meta(), recovering");
            e.into_inner()
        });
        cache
            .get_mut(path)
            .map(|cached| (Arc::clone(&cached.entries), cached.cached_at_ms))
    }

    /// Store directory entries in cache.
    /// `folder_cover` is stripped here (once at write time) instead of on
    /// every read, since covers are resolved separately via the cover pipeline.
    /// No fs::metadata() syscall — DriveWatcher handles invalidation.
    pub fn put(&self, path: PathBuf, mut entries: Vec<FileEntry>) {
        let mut cache = self.inner.lock().unwrap_or_else(|e| {
            log::warn!("[DIR-CACHE] Mutex poisoned in put(), recovering");
            e.into_inner()
        });
        for entry in &mut entries {
            entry.folder_cover = None;
        }
        let cached_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        cache.put(
            path,
            CachedFolder {
                entries: Arc::new(entries),
                cached_at_ms,
            },
        );
    }

    pub fn invalidate(&self, path: &PathBuf) {
        let mut cache = self.inner.lock().unwrap_or_else(|e| {
            log::warn!("[DIR-CACHE] Mutex poisoned in invalidate(), recovering");
            e.into_inner()
        });
        let _ = cache.pop(path);
    }

    pub fn invalidate_children(&self, parent: &PathBuf) {
        let mut cache = self.inner.lock().unwrap_or_else(|e| {
            log::warn!("[DIR-CACHE] Mutex poisoned in invalidate_children(), recovering");
            e.into_inner()
        });
        let keys_to_remove: Vec<PathBuf> = cache
            .iter()
            .filter(|(k, _)| k.starts_with(parent))
            .map(|(k, _)| k.clone())
            .collect();

        for key in keys_to_remove {
            cache.pop(&key);
        }
    }

    pub fn clear(&self) {
        let mut cache = self.inner.lock().unwrap_or_else(|e| {
            log::warn!("[DIR-CACHE] Mutex poisoned in clear(), recovering");
            e.into_inner()
        });
        cache.clear();
    }

    /// Returns the cache timestamp (Unix milliseconds) for a path without cloning entries.
    /// Useful for lightweight staleness checks (e.g., tab switch mtime validation).
    pub fn cached_at_ms(&self, path: &PathBuf) -> Option<u64> {
        let cache = self.inner.lock().unwrap_or_else(|e| {
            log::warn!("[DIR-CACHE] Mutex poisoned in cached_at_ms(), recovering");
            e.into_inner()
        });
        cache.peek(path).map(|cached| cached.cached_at_ms)
    }

    pub fn stats(&self) -> (usize, usize) {
        let cache = self.inner.lock().unwrap_or_else(|e| {
            log::warn!("[DIR-CACHE] Mutex poisoned in stats(), recovering");
            e.into_inner()
        });
        let total_items: usize = cache.iter().map(|(_, v)| v.entries.len()).sum();
        (cache.len(), total_items)
    }
}

impl Default for DirectoryCache {
    fn default() -> Self {
        Self::new()
    }
}
