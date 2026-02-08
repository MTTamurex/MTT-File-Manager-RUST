use crate::app::state::ImageViewerApp;
use std::path::{Path, PathBuf};
use std::sync::Arc;

impl ImageViewerApp {
    pub(super) fn normalize_for_match(p: &Path) -> String {
        let s = p.to_string_lossy().to_string().to_lowercase();
        if let Some(stripped) = s.strip_prefix(r"\\?\") {
            stripped.to_string()
        } else {
            s
        }
    }

    pub(super) fn clean_path(p: &Path) -> PathBuf {
        let s = p.to_string_lossy().to_string();
        if let Some(stripped) = s.strip_prefix(r"\\?\") {
            PathBuf::from(stripped)
        } else {
            p.to_path_buf()
        }
    }

    pub(super) fn invalidate_directory_caches(&mut self, path: &Path) {
        let path_buf = path.to_path_buf();
        self.directory_cache.invalidate(&path_buf);
        if let Some(di) = &self.directory_index {
            let _ = di.invalidate(path);
        }
    }

    pub(super) fn clear_tab_cache_for_normalized_path(&mut self, path_norm: &str) {
        for tab in self.tab_manager.tabs.iter_mut() {
            let tab_path = Self::normalize_for_match(Path::new(&tab.path));
            if tab_path == path_norm {
                tab.items = Arc::new(Vec::new());
                tab.all_items.clear();
            }
        }
    }
}
