use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

/// Shared cancellation marker for all requests for one version of a folder.
#[derive(Clone, Debug)]
pub struct FolderPreviewValidity(Arc<AtomicBool>);

impl Default for FolderPreviewValidity {
    fn default() -> Self {
        Self(Arc::new(AtomicBool::new(true)))
    }
}

impl FolderPreviewValidity {
    pub fn is_current(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Default)]
pub struct FolderPreviewRequests(HashMap<PathBuf, Weak<AtomicBool>>);

impl FolderPreviewRequests {
    pub fn begin(&mut self, path: &Path) -> FolderPreviewValidity {
        if let Some(token) = self.0.get(path).and_then(Weak::upgrade) {
            return FolderPreviewValidity(token);
        }
        // Only live jobs/results retain strong references; inactive folders do not accumulate.
        if self.0.len() >= 1024 {
            self.0.retain(|_, token| token.strong_count() > 0);
        }
        let token = FolderPreviewValidity::default();
        self.0.insert(path.to_path_buf(), Arc::downgrade(&token.0));
        token
    }

    pub fn invalidate(&mut self, path: &Path) {
        if let Some(token) = self.0.remove(path).and_then(|token| token.upgrade()) {
            token.store(false, Ordering::Release);
        }
    }

    pub fn invalidate_all(&mut self) {
        for (_, token) in self.0.drain() {
            if let Some(token) = token.upgrade() {
                token.store(false, Ordering::Release);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_preview_invalidation_cancels_old_results_not_new_requests() {
        let mut requests = FolderPreviewRequests::default();
        let old = requests.begin(Path::new("folder"));
        let same = requests.begin(Path::new("folder"));
        let unrelated = requests.begin(Path::new("other-folder"));
        requests.invalidate(Path::new("folder"));
        let new = requests.begin(Path::new("folder"));
        assert!(!old.is_current() && !same.is_current());
        assert!(new.is_current() && unrelated.is_current());
        requests.invalidate_all();
        assert!(!new.is_current() && !unrelated.is_current());
    }
}
