use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use lru::LruCache;
use parking_lot::Mutex;

use crate::domain::file_entry::FileEntry;

const CACHE_CAPACITY: usize = 120; // Bounded to avoid high long-session RAM growth
const MAX_TOTAL_CACHED_ITEMS: usize = 20_000; // Global cap to prevent a few huge folders from dominating RAM

struct CachedFolder {
    entries: Arc<Vec<FileEntry>>,
    cached_at_ms: u64,
}

struct DirectoryCacheInner {
    entries: LruCache<PathBuf, CachedFolder>,
    ordered_keys: BTreeSet<PathBuf>,
    total_items: usize,
}

impl DirectoryCacheInner {
    fn new() -> Self {
        Self {
            entries: LruCache::new(
                NonZeroUsize::new(CACHE_CAPACITY).expect("CACHE_CAPACITY must be non-zero"),
            ),
            ordered_keys: BTreeSet::new(),
            total_items: 0,
        }
    }

    fn total_items(&self) -> usize {
        self.total_items
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
    ///
    /// If this folder alone exceeds the global item budget
    /// (MAX_TOTAL_CACHED_ITEMS), the entry is NOT cached — it would dominate
    /// RAM with a single huge listing. Any previously cached copy for this
    /// path is removed.
    pub fn put(&self, path: PathBuf, mut entries: Vec<FileEntry>) {
        let mut cache = self.inner.lock();
        for entry in &mut entries {
            entry.folder_cover = None;
        }
        let cached_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // Reject oversized folders and remove any previously cached copy.
        if entries.len() > MAX_TOTAL_CACHED_ITEMS {
            if let Some(old) = cache.entries.pop(&path) {
                cache.total_items = cache.total_items.saturating_sub(old.entries.len());
            }
            cache.ordered_keys.remove(&path);
            return;
        }

        // Remove old entry first if replacing an existing key. This avoids
        // pre-evicting an unrelated LRU entry when the cache is already full.
        let replaced_existing = if let Some(old) = cache.entries.pop(&path) {
            cache.total_items = cache.total_items.saturating_sub(old.entries.len());
            true
        } else {
            false
        };

        // Ensure room in the LRU for new keys. Replacements already freed a slot.
        if !replaced_existing && cache.entries.len() >= CACHE_CAPACITY {
            if let Some((evicted_path, evicted)) = cache.entries.pop_lru() {
                cache.total_items = cache.total_items.saturating_sub(evicted.entries.len());
                cache.ordered_keys.remove(&evicted_path);
            }
        }

        let new_count = entries.len();
        cache.entries.put(
            path.clone(),
            CachedFolder {
                entries: Arc::new(entries),
                cached_at_ms,
            },
        );
        cache.ordered_keys.insert(path);
        cache.total_items += new_count;

        // Evict oldest entries until we're under the global item budget.
        while cache.total_items > MAX_TOTAL_CACHED_ITEMS && cache.entries.len() > 1 {
            let Some((evicted_path, evicted)) = cache.entries.pop_lru() else {
                break;
            };
            cache.total_items = cache.total_items.saturating_sub(evicted.entries.len());
            cache.ordered_keys.remove(&evicted_path);
        }
    }

    pub fn invalidate(&self, path: &PathBuf) {
        let mut cache = self.inner.lock();
        if let Some(old) = cache.entries.pop(path) {
            cache.total_items = cache.total_items.saturating_sub(old.entries.len());
        }
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
            if let Some(old) = cache.entries.pop(&key) {
                cache.total_items = cache.total_items.saturating_sub(old.entries.len());
            }
            cache.ordered_keys.remove(&key);
        }
    }

    pub fn clear(&self) {
        let mut cache = self.inner.lock();
        cache.entries.clear();
        cache.ordered_keys.clear();
        cache.total_items = 0;
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
    fn put_rejects_oversized_folder() {
        let cache = DirectoryCache::new();

        let huge_path = PathBuf::from(r"C:\huge");

        let huge_entries: Vec<FileEntry> = (0..MAX_TOTAL_CACHED_ITEMS + 1)
            .map(|idx| sample_entry(&format!(r"C:\huge\item{}", idx)))
            .collect();

        cache.put(huge_path.clone(), huge_entries);

        assert!(cache.get(&huge_path).is_none());
    }

    #[test]
    fn put_evicts_older_large_folders_when_total_item_budget_is_exceeded() {
        let cache = DirectoryCache::new();

        let older_path = PathBuf::from(r"C:\older");
        let newer_path = PathBuf::from(r"C:\newer");

        let older_entries: Vec<FileEntry> = (0..12_000)
            .map(|idx| sample_entry(&format!(r"C:\older\item{}", idx)))
            .collect();
        let newer_entries: Vec<FileEntry> = (0..12_000)
            .map(|idx| sample_entry(&format!(r"C:\newer\item{}", idx)))
            .collect();

        cache.put(older_path.clone(), older_entries);
        cache.put(newer_path.clone(), newer_entries);

        let (folders, total_items) = cache.stats();
        assert_eq!(folders, 1);
        assert_eq!(total_items, 12_000);
        assert!(cache.get(&older_path).is_none());
        assert!(cache.get(&newer_path).is_some());
    }

    #[test]
    fn total_items_tracks_put_and_invalidate() {
        let cache = DirectoryCache::new();

        let a = PathBuf::from(r"C:\a");
        let b = PathBuf::from(r"C:\b");

        cache.put(a.clone(), vec![sample_entry("a1"), sample_entry("a2")]);
        cache.put(b.clone(), vec![sample_entry("b1")]);

        assert_eq!(cache.stats(), (2, 3));

        cache.invalidate(&a);
        assert_eq!(cache.stats(), (1, 1));

        cache.invalidate(&b);
        assert_eq!(cache.stats(), (0, 0));
    }

    #[test]
    fn total_items_tracks_replacement() {
        let cache = DirectoryCache::new();

        let p = PathBuf::from(r"C:\dir");

        cache.put(
            p.clone(),
            vec![sample_entry("1"), sample_entry("2"), sample_entry("3")],
        );
        assert_eq!(cache.stats(), (1, 3));

        // Replace with fewer entries — total_items should decrease.
        cache.put(p.clone(), vec![sample_entry("1")]);
        assert_eq!(cache.stats(), (1, 1));
    }

    #[test]
    fn put_reject_oversized_removes_old_cached_copy() {
        let cache = DirectoryCache::new();
        let p = PathBuf::from(r"C:\dir");

        // First: cache a normal folder.
        cache.put(p.clone(), vec![sample_entry("x")]);
        assert_eq!(cache.stats(), (1, 1));

        // Second: replace with an oversized folder — old copy must be removed.
        let huge: Vec<FileEntry> = (0..MAX_TOTAL_CACHED_ITEMS + 1)
            .map(|i| sample_entry(&format!("item{}", i)))
            .collect();
        cache.put(p.clone(), huge);
        assert!(cache.get(&p).is_none());
        assert_eq!(cache.stats(), (0, 0));
    }

    #[test]
    fn replacing_lru_entry_when_full_does_not_double_subtract_total_items() {
        let cache = DirectoryCache::new();

        let lru_path = PathBuf::from(r"C:\dir0");
        for idx in 0..CACHE_CAPACITY {
            let path = PathBuf::from(format!(r"C:\dir{}", idx));
            cache.put(path, vec![sample_entry(&format!("item{}", idx))]);
        }
        assert_eq!(cache.stats(), (CACHE_CAPACITY, CACHE_CAPACITY));

        cache.put(
            lru_path.clone(),
            vec![sample_entry("replacement1"), sample_entry("replacement2")],
        );

        assert!(cache.get(&lru_path).is_some());
        assert_eq!(cache.stats(), (CACHE_CAPACITY, CACHE_CAPACITY + 1));
    }

    #[test]
    fn replacing_existing_entry_when_full_does_not_evict_unrelated_lru() {
        let cache = DirectoryCache::new();

        let lru_path = PathBuf::from(r"C:\dir0");
        let replaced_path = PathBuf::from(r"C:\dir50");
        for idx in 0..CACHE_CAPACITY {
            let path = PathBuf::from(format!(r"C:\dir{}", idx));
            cache.put(path, vec![sample_entry(&format!("item{}", idx))]);
        }
        assert_eq!(cache.stats(), (CACHE_CAPACITY, CACHE_CAPACITY));

        cache.put(replaced_path.clone(), vec![sample_entry("replacement")]);

        assert!(cache.get(&lru_path).is_some());
        assert!(cache.get(&replaced_path).is_some());
        assert_eq!(cache.stats(), (CACHE_CAPACITY, CACHE_CAPACITY));
    }
}
