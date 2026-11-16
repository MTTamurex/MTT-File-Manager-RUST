//! Operations for pinning/unpinning folders in Quick Access.

use crate::app::state::ImageViewerApp;
use crate::domain::pinned_folder::PinnedFolder;

impl ImageViewerApp {
    /// Pin a folder to Quick Access. No-op if already pinned.
    pub fn pin_folder(&mut self, path: &str) {
        let destination = self.resolve_network_location_path(std::path::Path::new(path));
        let path = destination.to_string_lossy();
        let path = path.as_ref();
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
        let pinned = PinnedFolder {
            path: path.to_string(),
            display_name: display_name.clone(),
            position,
        };

        self.app_state_db
            .save_pinned_folder(path, &display_name, position);
        self.pinned_folders.push(pinned);
    }

    /// Remove a folder from Quick Access.
    pub fn unpin_folder(&mut self, path: &str) {
        let destination = self.resolve_network_location_path(std::path::Path::new(path));
        let path = destination.to_string_lossy();
        let path = path.as_ref();
        self.pinned_folders.retain(|pf| pf.path != path);
        self.app_state_db.remove_pinned_folder(path);

        // Reassign positions sequentially after removal
        let ordered: Vec<String> = self
            .pinned_folders
            .iter()
            .map(|pf| pf.path.clone())
            .collect();
        for (i, pf) in self.pinned_folders.iter_mut().enumerate() {
            pf.position = i as i64;
        }
        self.app_state_db.update_pinned_positions(&ordered);
    }

    /// Remove pinned folders whose paths no longer exist on disk.
    /// Called after delete/move operations to keep Quick Access in sync.
    ///
    pub fn cleanup_deleted_pinned_folders(&mut self) {
        if self.file_operation_state.pinned_cleanup_receiver.is_some() {
            self.file_operation_state.pinned_cleanup_pending = true;
            return;
        }
        let paths: Vec<String> = self
            .pinned_folders
            .iter()
            .map(|pf| pf.path.clone())
            .collect();
        if paths.is_empty() {
            return;
        }

        let (tx, rx) = std::sync::mpsc::channel::<Vec<String>>();
        let ctx = self.ui_ctx.clone();
        if std::thread::Builder::new()
            .name("pinned-cleanup".into())
            .spawn(move || {
                let gone: Vec<String> = paths
                    .into_iter()
                    .filter(|p| {
                        // Perform the potentially blocking exists() check in this
                        // single background thread to avoid leaking per-path threads.
                        matches!(
                            std::fs::symlink_metadata(p),
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound
                        )
                    })
                    .collect();
                let _ = tx.send(gone);
                ctx.request_repaint();
            })
            .is_ok()
        {
            self.file_operation_state.pinned_cleanup_receiver = Some(rx);
        }
    }

    pub(crate) fn poll_deleted_pinned_folders(&mut self) {
        let Some(receiver) = &self.file_operation_state.pinned_cleanup_receiver else {
            return;
        };
        let result = receiver.try_recv();
        if matches!(result, Err(std::sync::mpsc::TryRecvError::Empty)) {
            return;
        }
        self.file_operation_state.pinned_cleanup_receiver = None;
        if let Ok(gone) = result {
            for path in gone {
                self.unpin_folder(&path);
            }
        }
        if std::mem::take(&mut self.file_operation_state.pinned_cleanup_pending) {
            self.cleanup_deleted_pinned_folders();
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
        let ordered: Vec<String> = self
            .pinned_folders
            .iter()
            .map(|pf| pf.path.clone())
            .collect();
        for (i, pf) in self.pinned_folders.iter_mut().enumerate() {
            pf.position = i as i64;
        }
        self.app_state_db.update_pinned_positions(&ordered);
    }
}
