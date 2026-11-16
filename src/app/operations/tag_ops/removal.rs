//! Indexed deletion/copy/GC cleanup; no global assignment cross-product.
use super::persistence::TagWrite;
use super::*;
use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn clear_tag_assignments_for_paths(&mut self, roots: &[PathBuf]) {
        let paths = self.tag_path_index.under_roots(roots);
        if paths.is_empty() {
            return;
        }
        if self.queue_tag_write(TagWrite::Clear(paths.clone())) {
            self.replace_tag_paths(paths.into_iter().map(|path| (path, Vec::new())).collect());
        }
    }

    pub fn clear_tag_assignments_for_copied_paths(&mut self, roots: &[PathBuf]) {
        if roots.is_empty() {
            return;
        }
        let paths = self.tag_path_index.under_roots(roots);
        let expected_rows = paths
            .iter()
            .filter_map(|path| self.tag_assignments.get(path))
            .map(Vec::len)
            .sum();
        let affected = if paths.is_empty() {
            roots.to_vec()
        } else {
            paths.clone()
        };
        if self.queue_tag_write(TagWrite::ClearCopied {
            paths: affected.clone(),
            expected_rows,
        }) {
            self.replace_tag_paths(paths.into_iter().map(|path| (path, Vec::new())).collect());
            // Send after the write barrier, so reapplication cannot be erased by
            // this queued clear. The worker performs filesystem probes, not the UI.
            self.reapply_auto_tag_rules_for_paths(&affected);
        }
    }

    pub fn reconcile_garbage_collected_tag_assignments(&mut self, paths: &[PathBuf]) {
        let mut keys = FxHashSet::default();
        for path in paths {
            keys.extend(self.tag_path_index.exact(path).cloned());
        }
        if keys.is_empty() {
            return;
        }
        self.replace_tag_paths(keys.into_iter().map(|path| (path, Vec::new())).collect());
        self.prune_paths_from_loaded_tag_views(paths);
    }
}
