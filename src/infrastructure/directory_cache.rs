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
                NonZeroUsize::new(CACHE_CAPACITY).expect("CACHE_CAPACITY must be non-zero"),
            ))),
        }
    }

    /// Returns cached entries immediately if available.
    /// Cache is invalidated by: DriveWatcher (when enabled), per-folder
    /// notify-watcher, consistency probe, and mtime validation in fast_paths.
    ///
    /// NOTE: `folder_cover` is stripped on read — it is resolved separately
    /// via the cover pipeline (SQLite + existence check + cover worker) to
    /// avoid returning stale covers from a previous visit.
    pub fn get(&self, path: &PathBuf) -> Option<Vec<FileEntry>> {
        let mut cache = self.inner.lock().ok()?;
        if let Some(cached) = cache.get_mut(path) {
            let mut entries = cached.entries.clone();
            for entry in &mut entries {
                entry.folder_cover = None;
            }
            return Some(entries);
        }
        None
    }

    /// Returns cached entries and the cache timestamp in Unix milliseconds.
    pub fn get_with_meta(&self, path: &PathBuf) -> Option<(Vec<FileEntry>, u64)> {
        let mut cache = self.inner.lock().ok()?;
        if let Some(cached) = cache.get_mut(path) {
            let mut entries = cached.entries.clone();
            for entry in &mut entries {
                entry.folder_cover = None;
            }
            return Some((entries, cached.cached_at_ms));
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

    /// Returns the cache timestamp (Unix milliseconds) for a path without cloning entries.
    /// Useful for lightweight staleness checks (e.g., tab switch mtime validation).
    pub fn cached_at_ms(&self, path: &PathBuf) -> Option<u64> {
        let cache = self.inner.lock().ok()?;
        cache.peek(path).map(|cached| cached.cached_at_ms)
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
