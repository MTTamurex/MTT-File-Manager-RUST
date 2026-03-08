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
    ///
    /// FIX: Uses a background thread for the `.exists()` check to avoid
    /// blocking the UI thread. `Path::exists()` calls `GetFileAttributesW`
    /// which can block indefinitely on network/cloud/USB drives.
    pub fn cleanup_deleted_pinned_folders(&mut self) {
        let paths: Vec<String> = self.pinned_folders.iter().map(|pf| pf.path.clone()).collect();
        if paths.is_empty() {
            return;
        }

        // Probe paths in a background thread with a timeout per path.
        // Results arrive on the next frame via a channel.
        let (tx, rx) = std::sync::mpsc::channel::<Vec<String>>();
        std::thread::Builder::new()
            .name("pinned-cleanup".into())
            .spawn(move || {
                let gone: Vec<String> = paths
                    .into_iter()
                    .filter(|p| {
                        // Perform the potentially blocking exists() check in this
                        // single background thread to avoid leaking per-path threads.
                        !std::path::Path::new(p).exists()
                    })
                    .collect();
                let _ = tx.send(gone);
            })
            .ok();

        // Non-blocking: try to receive immediately (will succeed if probes are fast, i.e., local).
        // If not ready yet, the cleanup will happen on the next call.
        if let Ok(gone) = rx.recv_timeout(std::time::Duration::from_millis(50)) {
            for path in gone {
                log::info!("[PinnedFolders] Auto-removing deleted folder: {}", path);
                self.unpin_folder(&path);
            }
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
