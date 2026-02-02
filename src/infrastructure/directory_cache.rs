use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use lru::LruCache;

use crate::domain::file_entry::FileEntry;

const CACHE_CAPACITY: usize = 500; // 500 items (approx 83-167MB RAM)
const REVALIDATE_DEBOUNCE: Duration = Duration::from_millis(2000); // 2 seconds

struct CachedFolder {
    entries: Vec<FileEntry>,
    last_check: Instant,        // When we last checked the disk for changes
    last_modified: SystemTime,  // The folder's modification time on disk
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

    /// Phase 1: Instant Feedback (The Cache Hit)
    /// Returns cached entries immediately if available
    pub fn get(&self, path: &PathBuf) -> Option<Vec<FileEntry>> {
        let mut cache = self.inner.lock().ok()?;
        if let Some(cached) = cache.get_mut(path) {
            return Some(cached.entries.clone());
        }
        None
    }

    /// Phase 2: The Debounce Check (Stale-While-Revalidate)
    /// Checks if cache needs revalidation based on debounce time
    pub fn needs_revalidation(&self, path: &PathBuf) -> bool {
        if let Ok(mut cache) = self.inner.lock() {
            if let Some(cached) = cache.get_mut(path) {
                let elapsed = cached.last_check.elapsed();
                let needs_reval = elapsed > REVALIDATE_DEBOUNCE;
                eprintln!("[STALE-WHILE-REVALIDATE] Debounce check for {:?}: {}ms elapsed, needs_revalidation: {}", 
                    path, elapsed.as_millis(), needs_reval);
                return needs_reval;
            }
        }
        false
    }

    /// Get current modification time of directory on disk
    pub fn get_dir_mtime(&self, path: &PathBuf) -> Option<SystemTime> {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
    }

    /// Check if directory has been modified since last cache
    pub fn has_directory_changed(&self, path: &PathBuf) -> Option<bool> {
        if let Ok(mut cache) = self.inner.lock() {
            if let Some(cached) = cache.get_mut(path) {
                if let Some(current_mtime) = self.get_dir_mtime(path) {
                    let has_changed = cached.last_modified != current_mtime;
                    eprintln!("[STALE-WHILE-REVALIDATE] Directory change check for {:?}: cached_time={:?}, current_time={:?}, changed={}", 
                        path, cached.last_modified, current_mtime, has_changed);
                    return Some(has_changed);
                }
            }
        }
        None
    }

    /// Phase 3: Store/update cache with fresh data
    /// Stores directory entries with current modification time
    pub fn put(&self, path: PathBuf, entries: Vec<FileEntry>) {
        // Read mtime from disk (used when caller doesn't have it)
        let last_modified = self.get_dir_mtime(&path).unwrap_or_else(SystemTime::now);
        self.put_with_mtime(path, entries, last_modified);
    }

    /// Store cache entries with a known modification time (avoids extra metadata syscall)
    pub fn put_with_mtime(&self, path: PathBuf, entries: Vec<FileEntry>, last_modified: SystemTime) {
        if let Ok(mut cache) = self.inner.lock() {
            eprintln!("[STALE-WHILE-REVALIDATE] Storing {} entries in cache for {:?} with mtime={:?}",
                entries.len(), path, last_modified);
            cache.put(
                path,
                CachedFolder {
                    entries,
                    last_check: Instant::now(),
                    last_modified,
                },
            );
        }
    }

    /// Update last_check time without reloading entries
    pub fn update_check_time(&self, path: &PathBuf) {
        if let Ok(mut cache) = self.inner.lock() {
            if let Some(cached) = cache.get_mut(path) {
                cached.last_check = Instant::now();
                eprintln!("[STALE-WHILE-REVALIDATE] Updated check time for {:?} - HDD silence maintained", path);
            }
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
