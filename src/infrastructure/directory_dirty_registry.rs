use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

#[derive(Clone, Default)]
pub struct DirectoryDirtyRegistry {
    inner: Arc<Mutex<HashMap<PathBuf, u64>>>,
    next_version: Arc<AtomicU64>,
}

impl DirectoryDirtyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_dirty(&self, path: &Path) -> bool {
        self.inner.lock().contains_key(path)
    }

    pub fn mark_dirty(&self, path: &Path) -> u64 {
        let path_buf = path.to_path_buf();
        let next_version = self
            .next_version
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        let mut entries = self.inner.lock();
        entries.insert(path_buf, next_version);
        next_version
    }

    pub fn version(&self, path: &Path) -> Option<u64> {
        self.inner.lock().get(path).copied()
    }

    pub fn clear_dirty_if_version(&self, path: &Path, expected_version: Option<u64>) -> bool {
        let Some(expected_version) = expected_version else {
            return false;
        };
        let mut entries = self.inner.lock();
        if entries.get(path).copied() != Some(expected_version) {
            return false;
        }
        entries.remove(path);
        true
    }

    pub fn clear_dirty(&self, path: &Path) {
        let mut entries = self.inner.lock();
        let _ = entries.remove(path);
    }

    /// Number of currently tracked dirty entries (for diagnostics only).
    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }

    /// Returns `true` if no dirty entries are tracked.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_scan_cannot_clear_newer_dirty_version() {
        let registry = DirectoryDirtyRegistry::new();
        let path = Path::new(r"D:\Source");
        let old_version = registry.mark_dirty(path);
        let new_version = registry.mark_dirty(path);

        assert!(!registry.clear_dirty_if_version(path, Some(old_version)));
        assert_eq!(registry.version(path), Some(new_version));
        assert!(registry.clear_dirty_if_version(path, Some(new_version)));
        assert!(!registry.is_dirty(path));
    }

    #[test]
    fn scan_started_clean_cannot_clear_later_invalidation() {
        let registry = DirectoryDirtyRegistry::new();
        let path = Path::new(r"D:\Source");
        let version_at_scan_start = registry.version(path);
        let invalidation_version = registry.mark_dirty(path);

        assert!(!registry.clear_dirty_if_version(path, version_at_scan_start));
        assert_eq!(registry.version(path), Some(invalidation_version));
    }

    #[test]
    fn dirty_versions_are_not_reused_after_clear() {
        let registry = DirectoryDirtyRegistry::new();
        let path = Path::new(r"D:\Source");
        let first_version = registry.mark_dirty(path);
        assert!(registry.clear_dirty_if_version(path, Some(first_version)));

        let second_version = registry.mark_dirty(path);

        assert_ne!(first_version, second_version);
        assert!(!registry.clear_dirty_if_version(path, Some(first_version)));
        assert_eq!(registry.version(path), Some(second_version));
    }
}
