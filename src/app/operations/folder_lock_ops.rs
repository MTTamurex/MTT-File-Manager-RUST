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
            self.app_state_db.remove_folder_lock(&path);
            self.current_folder_locked = false;
        } else {
            // Lock: capture current view settings
            let lock = FolderLock {
                view_mode: self.view_mode,
                sort_mode: self.sort_mode,
                sort_descending: self.sort_descending,
                folders_position: self.folders_position,
            };
            self.app_state_db.save_folder_lock(&path, &lock);
            self.folder_locks.insert(path, lock);
            self.current_folder_locked = true;
        }
    }

    /// Called after navigating to a new folder.
    /// If the destination has a lock, applies the locked settings.
    /// If not, restores the "normal" (unlocked) settings so that locked-folder
    /// overrides don't bleed into unlocked folders.
    pub fn apply_folder_lock_if_present(&mut self) {
        let lock_start = std::time::Instant::now();
        let path = &self.navigation_state.current_path;
        if let Some(lock) = self.folder_locks.get(path).cloned() {
            log::info!(
                "[FOLDER-LOCK] Applying lock for {:?}: view={:?}, sort={:?}, desc={}, pos={:?}",
                path,
                lock.view_mode,
                lock.sort_mode,
                lock.sort_descending,
                lock.folders_position
            );
            self.view_mode = lock.view_mode;
            self.sort_mode = lock.sort_mode;
            self.sort_descending = lock.sort_descending;
            self.folders_position = lock.folders_position;
            self.current_folder_locked = true;
        } else {
            log::debug!(
                "[FOLDER-LOCK] No lock for {:?} (known locks: {})",
                path,
                self.folder_locks.len()
            );
            // Restore normal (unlocked) settings
            self.view_mode = self.view_mode_normal;
            self.sort_mode = self.sort_mode_normal;
            self.sort_descending = self.sort_descending_normal;
            self.folders_position = self.folders_position_normal;
            self.current_folder_locked = false;
        }

        let lock_ms = lock_start.elapsed().as_millis();
        if lock_ms > 20 {
            log::warn!(
                "[PERF-FOLDER-LOCK] apply_folder_lock_if_present took {}ms path={:?} locked={} known_locks={}",
                lock_ms,
                path,
                self.current_folder_locked,
                self.folder_locks.len(),
            );
        }
    }

    /// Like `apply_folder_lock_if_present`, but does **not** reset view/sort
    /// settings to the global "normal" defaults when no lock exists.
    ///
    /// Used when restoring a tab: the tab's own stored sort/view preferences
    /// must be preserved for unlocked folders. Only a lock override (if one
    /// exists for the current path) should win over the tab's saved state.
    pub fn apply_folder_lock_on_tab_restore(&mut self) {
        let path = &self.navigation_state.current_path;
        if let Some(lock) = self.folder_locks.get(path).cloned() {
            log::info!(
                "[FOLDER-LOCK] Applying lock (tab restore) for {:?}: view={:?}, sort={:?}, desc={}, pos={:?}",
                path,
                lock.view_mode,
                lock.sort_mode,
                lock.sort_descending,
                lock.folders_position
            );
            self.view_mode = lock.view_mode;
            self.sort_mode = lock.sort_mode;
            self.sort_descending = lock.sort_descending;
            self.folders_position = lock.folders_position;
            self.current_folder_locked = true;
        } else {
            log::debug!(
                "[FOLDER-LOCK] No lock for {:?} on tab restore - preserving tab sort/view settings",
                path
            );
            // Do NOT reset to sort_mode_normal / view_mode_normal here.
            // The tab's own saved sort_mode/view_mode are already loaded into the
            // app state by sync_from_tab() before this call.
            self.current_folder_locked = false;
        }
    }
}
