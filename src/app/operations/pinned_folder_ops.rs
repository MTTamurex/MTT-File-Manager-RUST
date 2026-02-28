//! Operations for pinning/unpinning folders in Quick Access.

use crate::app::state::ImageViewerApp;
use crate::domain::pinned_folder::PinnedFolder;

impl ImageViewerApp {
    /// Pin a folder to Quick Access. No-op if already pinned.
    pub fn pin_folder(&mut self, path: &str) {
        // Avoid duplicates
        if self.pinned_folders.iter().any(|pf| pf.path == path) {
            return;
        }

        let display_name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string();

        let position = self.pinned_folders.len() as i64;
        let pinned = PinnedFolder { path: path.to_string(), display_name: display_name.clone(), position };

        self.disk_cache.save_pinned_folder(path, &display_name, position);
        self.pinned_folders.push(pinned);
    }

    /// Remove a folder from Quick Access.
    pub fn unpin_folder(&mut self, path: &str) {
        self.pinned_folders.retain(|pf| pf.path != path);
        self.disk_cache.remove_pinned_folder(path);

        // Reassign positions sequentially after removal
        let ordered: Vec<String> = self.pinned_folders.iter().map(|pf| pf.path.clone()).collect();
        for (i, pf) in self.pinned_folders.iter_mut().enumerate() {
            pf.position = i as i64;
        }
        self.disk_cache.update_pinned_positions(&ordered);
    }

    /// Remove pinned folders whose paths no longer exist on disk.
    /// Called after delete/move operations to keep Quick Access in sync.
    pub fn cleanup_deleted_pinned_folders(&mut self) {
        let gone: Vec<String> = self
            .pinned_folders
            .iter()
            .filter(|pf| !std::path::Path::new(&pf.path).exists())
            .map(|pf| pf.path.clone())
            .collect();

        for path in gone {
            log::info!("[PinnedFolders] Auto-removing deleted folder: {}", path);
            self.unpin_folder(&path);
        }
    }

    /// Reorder pinned folders by moving item at `from` to position `to`.
    pub fn reorder_pinned_folder(&mut self, from: usize, to: usize) {
        if from == to || from >= self.pinned_folders.len() {
            return;
        }

        let item = self.pinned_folders.remove(from);
        let insert_at = to.min(self.pinned_folders.len());
        self.pinned_folders.insert(insert_at, item);

        // Update positions in memory and DB
        let ordered: Vec<String> = self.pinned_folders.iter().map(|pf| pf.path.clone()).collect();
        for (i, pf) in self.pinned_folders.iter_mut().enumerate() {
            pf.position = i as i64;
        }
        self.disk_cache.update_pinned_positions(&ordered);
    }
}
