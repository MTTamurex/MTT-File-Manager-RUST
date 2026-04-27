use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use lru::LruCache;
use parking_lot::Mutex;

use crate::domain::file_entry::FileEntry;

const CACHE_CAPACITY: usize = 200; // Bounded to avoid high long-session RAM growth
const MAX_TOTAL_CACHED_ITEMS: usize = 50_000; // Global cap to prevent a few huge folders from dominating RAM

struct CachedFolder {
    entries: Arc<Vec<FileEntry>>,
    cached_at_ms: u64,
}

struct DirectoryCacheInner {
    entries: LruCache<PathBuf, CachedFolder>,
    ordered_keys: BTreeSet<PathBuf>,
}

impl DirectoryCacheInner {
    fn new() -> Self {
        Self {
            entries: LruCache::new(
                NonZeroUsize::new(CACHE_CAPACITY).expect("CACHE_CAPACITY must be non-zero"),
            ),
            ordered_keys: BTreeSet::new(),
        }
    }

    fn sync_ordered_keys(&mut self) {
        self.ordered_keys = self.entries.iter().map(|(path, _)| path.clone()).collect();
    }

    fn total_items(&self) -> usize {
        self.entries.iter().map(|(_, cached)| cached.entries.len()).sum()
    }
}

pub struct DirectoryCache {
    inner: Arc<Mutex<DirectoryCacheInner>>,
}

impl DirectoryCache {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(DirectoryCacheInner::new())),
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
        let mut cache = self.inner.lock();
        cache
            .entries
            .get_mut(path)
            .map(|cached| Arc::clone(&cached.entries))
    }

    /// Returns cached entries and the cache timestamp in Unix milliseconds.
    pub fn get_with_meta(&self, path: &PathBuf) -> Option<(Arc<Vec<FileEntry>>, u64)> {
        let mut cache = self.inner.lock();
        cache
            .entries
            .get_mut(path)
            .map(|cached| (Arc::clone(&cached.entries), cached.cached_at_ms))
    }

    /// Store directory entries in cache.
    /// `folder_cover` is stripped here (once at write time) instead of on
    /// every read, since covers are resolved separately via the cover pipeline.
    /// No fs::metadata() syscall — DriveWatcher handles invalidation.
    pub fn put(&self, path: PathBuf, mut entries: Vec<FileEntry>) {
        let mut cache = self.inner.lock();
        for entry in &mut entries {
            entry.folder_cover = None;
        }
        let cached_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        cache.entries.put(
            path,
            CachedFolder {
                entries: Arc::new(entries),
                cached_at_ms,
            },
        );

        let mut total_items = cache.total_items();
        while total_items > MAX_TOTAL_CACHED_ITEMS && cache.entries.len() > 1 {
            let Some((evicted_path, evicted)) = cache.entries.pop_lru() else {
                break;
            };
            total_items = total_items.saturating_sub(evicted.entries.len());
            cache.ordered_keys.remove(&evicted_path);
        }

        cache.sync_ordered_keys();
    }

    pub fn invalidate(&self, path: &PathBuf) {
        let mut cache = self.inner.lock();
        let _ = cache.entries.pop(path);
        cache.ordered_keys.remove(path);
    }

    pub fn invalidate_children(&self, parent: &PathBuf) {
        let mut cache = self.inner.lock();
        let keys_to_remove: Vec<PathBuf> = cache
            .ordered_keys
            .range(parent.clone()..)
            .take_while(|path| path.starts_with(parent))
            .cloned()
            .collect();

        for key in keys_to_remove {
            cache.entries.pop(&key);
            cache.ordered_keys.remove(&key);
        }
    }

    pub fn clear(&self) {
        let mut cache = self.inner.lock();
        cache.entries.clear();
        cache.ordered_keys.clear();
    }

    /// Returns the cache timestamp (Unix milliseconds) for a path without cloning entries.
    /// Useful for lightweight staleness checks (e.g., tab switch mtime validation).
    pub fn cached_at_ms(&self, path: &PathBuf) -> Option<u64> {
        let cache = self.inner.lock();
        cache.entries.peek(path).map(|cached| cached.cached_at_ms)
    }

    pub fn stats(&self) -> (usize, usize) {
        let cache = self.inner.lock();
        (cache.entries.len(), cache.total_items())
    }
}

impl Default for DirectoryCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(path: &str) -> FileEntry {
        FileEntry::from_path(PathBuf::from(path), true)
    }

    #[test]
    fn invalidate_children_removes_only_matching_subtree() {
        let cache = DirectoryCache::new();

        let root = PathBuf::from(r"C:\root");
        let child = PathBuf::from(r"C:\root\child");
        let nested = PathBuf::from(r"C:\root\child\nested");
        let sibling = PathBuf::from(r"C:\root\other");
        let outside = PathBuf::from(r"D:\elsewhere");

        for path in [&root, &child, &nested, &sibling, &outside] {
            cache.put(
                path.clone(),
                vec![sample_entry(path.to_string_lossy().as_ref())],
            );
        }

        cache.invalidate_children(&child);

        assert!(cache.get(&root).is_some());
        assert!(cache.get(&child).is_none());
        assert!(cache.get(&nested).is_none());
        assert!(cache.get(&sibling).is_some());
        assert!(cache.get(&outside).is_some());
    }

    #[test]
    fn put_evicts_older_large_folders_when_total_item_budget_is_exceeded() {
        let cache = DirectoryCache::new();

        let older_path = PathBuf::from(r"C:\older");
        let newer_path = PathBuf::from(r"C:\newer");

        let older_entries: Vec<FileEntry> = (0..30_000)
            .map(|idx| sample_entry(&format!(r"C:\older\item{}", idx)))
            .collect();
        let newer_entries: Vec<FileEntry> = (0..30_000)
            .map(|idx| sample_entry(&format!(r"C:\newer\item{}", idx)))
            .collect();

        cache.put(older_path.clone(), older_entries);
        cache.put(newer_path.clone(), newer_entries);

        let (folders, total_items) = cache.stats();
        assert_eq!(folders, 1);
        assert_eq!(total_items, 30_000);
        assert!(cache.get(&older_path).is_none());
        assert!(cache.get(&newer_path).is_some());
    }
}
