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
            .map(|entries| entries.contains_key(path))
            .unwrap_or(true)
    }

    pub fn mark_dirty(&self, path: &Path) -> u64 {
        let path_buf = path.to_path_buf();
        if let Ok(mut entries) = self.inner.lock() {
            let next_version = entries
                .get(&path_buf)
                .copied()
                .unwrap_or(0)
                .saturating_add(1);
            entries.insert(path_buf, next_version);
            next_version
        } else {
            0
        }
    }

    pub fn clear_dirty(&self, path: &Path) {
        if let Ok(mut entries) = self.inner.lock() {
            let _ = entries.remove(path);
        }
    }
}