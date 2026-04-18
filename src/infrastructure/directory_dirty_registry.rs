use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct DirectoryDirtyRegistry {
    inner: Arc<Mutex<HashMap<PathBuf, u64>>>,
}

impl DirectoryDirtyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_dirty(&self, path: &Path) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|e| {
                log::warn!("[DIRTY-REGISTRY] Mutex poisoned in is_dirty(), recovering");
                e.into_inner()
            })
            .contains_key(path)
    }

    pub fn mark_dirty(&self, path: &Path) -> u64 {
        let path_buf = path.to_path_buf();
        let mut entries = self.inner.lock().unwrap_or_else(|e| {
            log::warn!("[DIRTY-REGISTRY] Mutex poisoned in mark_dirty(), recovering");
            e.into_inner()
        });
        let next_version = entries
            .get(&path_buf)
            .copied()
            .unwrap_or(0)
            .saturating_add(1);
        entries.insert(path_buf, next_version);
        next_version
    }

    pub fn clear_dirty(&self, path: &Path) {
        let mut entries = self.inner.lock().unwrap_or_else(|e| {
            log::warn!("[DIRTY-REGISTRY] Mutex poisoned in clear_dirty(), recovering");
            e.into_inner()
        });
        let _ = entries.remove(path);
    }
}