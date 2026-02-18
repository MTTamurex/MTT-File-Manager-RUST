use crate::app::state::ImageViewerApp;
use crate::domain::folder_lock::FolderLock;

impl ImageViewerApp {
    /// Toggle the lock for the current folder.
    /// If unlocked: capture current view settings and save to DB.
    /// If locked: remove from DB and re-enable controls.
    pub fn toggle_folder_lock(&mut self) {
        let path = self.navigation_state.current_path.clone();
        if path.is_empty()
            || self.navigation_state.is_computer_view
            || self.navigation_state.is_recycle_bin_view
        {
            return;
        }

        if self.current_folder_locked {
            // Unlock
            self.folder_locks.remove(&path);
            self.disk_cache.remove_folder_lock(&path);
            self.current_folder_locked = false;
        } else {
            // Lock: capture current view settings
            let lock = FolderLock {
                view_mode: self.view_mode,
                sort_mode: self.sort_mode,
                sort_descending: self.sort_descending,
                folders_position: self.folders_position,
            };
            self.disk_cache.save_folder_lock(&path, &lock);
            self.folder_locks.insert(path, lock);
            self.current_folder_locked = true;
        }
    }

    /// Called after navigating to a new folder.
    /// If the destination has a lock, applies the locked settings.
    /// If not, restores the "normal" (unlocked) settings so that locked-folder
    /// overrides don't bleed into unlocked folders.
    pub fn apply_folder_lock_if_present(&mut self) {
        let path = &self.navigation_state.current_path;
        if let Some(lock) = self.folder_locks.get(path).cloned() {
            log::info!("[FOLDER-LOCK] Applying lock for {:?}: view={:?}, sort={:?}, desc={}, pos={:?}",
                path, lock.view_mode, lock.sort_mode, lock.sort_descending, lock.folders_position);
            self.view_mode = lock.view_mode;
            self.sort_mode = lock.sort_mode;
            self.sort_descending = lock.sort_descending;
            self.folders_position = lock.folders_position;
            self.current_folder_locked = true;
        } else {
            log::debug!("[FOLDER-LOCK] No lock for {:?} (known locks: {})", path, self.folder_locks.len());
            // Restore normal (unlocked) settings
            self.view_mode = self.view_mode_normal;
            self.sort_mode = self.sort_mode_normal;
            self.sort_descending = self.sort_descending_normal;
            self.folders_position = self.folders_position_normal;
            self.current_folder_locked = false;
        }
    }
}
