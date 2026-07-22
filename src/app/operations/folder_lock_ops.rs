use crate::app::state::ImageViewerApp;
use crate::domain::folder_lock::{FolderLock, FolderLockScope};
use crate::domain::special_paths::{is_tag_view_path, is_virtual_path, COMPUTER_VIEW_ID};
use rust_i18n::t;
use std::collections::HashMap;
use std::path::Path;

pub(crate) fn is_lockable_view_path(path: &str) -> bool {
    !path.is_empty()
        && (path == COMPUTER_VIEW_ID || is_tag_view_path(path) || !is_virtual_path(path))
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedFolderLock {
    pub source_path: String,
    pub lock: FolderLock,
}

fn resolve_folder_lock<'a>(
    folder_locks: &'a HashMap<String, FolderLock>,
    path: &str,
) -> Option<(&'a str, &'a FolderLock)> {
    if !is_lockable_view_path(path) {
        return None;
    }

    if let Some((source_path, lock)) = folder_locks.get_key_value(path) {
        return Some((source_path.as_str(), lock));
    }

    if is_virtual_path(path) {
        return None;
    }

    Path::new(path).ancestors().skip(1).find_map(|ancestor| {
        let ancestor = ancestor.to_str()?;
        let (source_path, lock) = folder_locks.get_key_value(ancestor)?;
        (lock.scope == FolderLockScope::Descendants).then_some((source_path.as_str(), lock))
    })
}

impl ImageViewerApp {
    pub(crate) fn current_folder_lock_resolution(&self) -> Option<ResolvedFolderLock> {
        resolve_folder_lock(&self.folder_locks, &self.navigation_state.current_path).map(
            |(source_path, lock)| ResolvedFolderLock {
                source_path: source_path.to_owned(),
                lock: lock.clone(),
            },
        )
    }

    /// Capture and persist the current view settings for the selected scope.
    pub fn set_current_folder_lock(&mut self, scope: FolderLockScope) -> bool {
        let path = self.navigation_state.current_path.clone();
        if !is_lockable_view_path(&path)
            || (scope == FolderLockScope::Descendants && is_virtual_path(&path))
        {
            return false;
        }

        let lock = FolderLock {
            view_mode: self.view_mode,
            sort_mode: self.sort_mode,
            sort_descending: self.sort_descending,
            folders_position: self.folders_position,
            scope,
        };
        if let Err(error) = self.app_state_db.save_folder_lock(&path, &lock) {
            log::error!("[FOLDER-LOCK] Failed to save lock for {path:?}: {error}");
            self.notifications.error(
                t!(
                    "secondary_toolbar.lock_change_failed",
                    error = error.to_string()
                )
                .to_string(),
            );
            return false;
        }
        self.folder_locks.insert(path, lock);
        self.current_folder_locked = true;
        true
    }

    /// Remove only the lock defined directly on the current folder.
    pub fn remove_current_folder_lock(&mut self) -> bool {
        let path = self.navigation_state.current_path.clone();
        if !self.folder_locks.contains_key(&path) {
            return false;
        }

        if let Err(error) = self.app_state_db.remove_folder_lock(&path) {
            log::error!("[FOLDER-LOCK] Failed to remove lock for {path:?}: {error}");
            self.notifications.error(
                t!(
                    "secondary_toolbar.lock_change_failed",
                    error = error.to_string()
                )
                .to_string(),
            );
            return false;
        }

        self.folder_locks.remove(&path);
        if let Some(resolved) = self.current_folder_lock_resolution() {
            self.apply_resolved_folder_lock(&resolved);
        } else {
            self.current_folder_locked = false;
        }
        true
    }

    fn apply_resolved_folder_lock(&mut self, resolved: &ResolvedFolderLock) {
        self.view_mode = resolved.lock.view_mode;
        self.sort_mode = resolved.lock.sort_mode;
        self.sort_descending = resolved.lock.sort_descending;
        self.folders_position = resolved.lock.folders_position;
        self.current_folder_locked = true;
    }

    /// Called after navigating to a new folder.
    /// If the destination has a lock, applies the locked settings.
    /// If not, restores the "normal" (unlocked) settings so that locked-folder
    /// overrides don't bleed into unlocked folders.
    pub fn apply_folder_lock_if_present(&mut self) {
        let lock_start = std::time::Instant::now();
        let path = self.navigation_state.current_path.clone();
        if !is_lockable_view_path(&path) {
            self.current_folder_locked = false;
            return;
        }
        if let Some(resolved) = self.current_folder_lock_resolution() {
            log::info!(
                "[FOLDER-LOCK] Applying lock for {:?} from {:?}: view={:?}, sort={:?}, desc={}, pos={:?}, scope={:?}",
                path,
                resolved.source_path,
                resolved.lock.view_mode,
                resolved.lock.sort_mode,
                resolved.lock.sort_descending,
                resolved.lock.folders_position,
                resolved.lock.scope,
            );
            self.apply_resolved_folder_lock(&resolved);
        } else {
            log::debug!(
                "[FOLDER-LOCK] No lock for {:?} (known locks: {})",
                path,
                self.folder_locks.len()
            );
            // Restore normal (unlocked) settings. In dual panel mode, each
            // panel owns its current view mode; using the global normal value
            // here would make navigation adopt whichever panel changed view
            // mode last.
            if !self.dual_panel_enabled || self.current_folder_locked {
                self.view_mode = self.view_mode_normal;
            }
            self.sort_mode = if path == COMPUTER_VIEW_ID {
                self.sort_mode_computer
            } else {
                self.sort_mode_normal
            };
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
        let path = self.navigation_state.current_path.clone();
        if !is_lockable_view_path(&path) {
            self.current_folder_locked = false;
            return;
        }
        if let Some(resolved) = self.current_folder_lock_resolution() {
            log::info!(
                "[FOLDER-LOCK] Applying lock (tab restore) for {:?} from {:?}: view={:?}, sort={:?}, desc={}, pos={:?}, scope={:?}",
                path,
                resolved.source_path,
                resolved.lock.view_mode,
                resolved.lock.sort_mode,
                resolved.lock.sort_descending,
                resolved.lock.folders_position,
                resolved.lock.scope,
            );
            self.apply_resolved_folder_lock(&resolved);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::file_entry::{FoldersPosition, SortMode, ViewMode};

    fn lock(scope: FolderLockScope) -> FolderLock {
        FolderLock {
            view_mode: ViewMode::Grid,
            sort_mode: SortMode::Name,
            sort_descending: false,
            folders_position: FoldersPosition::First,
            scope,
        }
    }

    #[test]
    fn exact_lock_wins_over_inherited_lock() {
        let mut locks = HashMap::new();
        locks.insert(r"C:\Photos".to_owned(), lock(FolderLockScope::Descendants));
        locks.insert(
            r"C:\Photos\Trips".to_owned(),
            lock(FolderLockScope::CurrentFolder),
        );

        let (source, _) = resolve_folder_lock(&locks, r"C:\Photos\Trips").unwrap();

        assert_eq!(source, r"C:\Photos\Trips");
    }

    #[test]
    fn nearest_descendant_lock_is_inherited() {
        let mut locks = HashMap::new();
        locks.insert(r"C:\Photos".to_owned(), lock(FolderLockScope::Descendants));
        locks.insert(
            r"C:\Photos\Trips".to_owned(),
            lock(FolderLockScope::Descendants),
        );

        let (source, _) = resolve_folder_lock(&locks, r"C:\Photos\Trips\2026\July").unwrap();

        assert_eq!(source, r"C:\Photos\Trips");
    }

    #[test]
    fn current_folder_scope_is_not_inherited() {
        let mut locks = HashMap::new();
        locks.insert(
            r"C:\Photos".to_owned(),
            lock(FolderLockScope::CurrentFolder),
        );

        assert!(resolve_folder_lock(&locks, r"C:\Photos\Trips").is_none());
    }

    #[test]
    fn similarly_prefixed_path_is_not_a_descendant() {
        let mut locks = HashMap::new();
        locks.insert(r"C:\Photos".to_owned(), lock(FolderLockScope::Descendants));

        assert!(resolve_folder_lock(&locks, r"C:\Photos Archive\Trips").is_none());
    }

    #[test]
    fn virtual_views_do_not_inherit_locks() {
        let mut locks = HashMap::new();
        locks.insert(
            COMPUTER_VIEW_ID.to_owned(),
            lock(FolderLockScope::Descendants),
        );

        assert!(resolve_folder_lock(&locks, "::computer::child").is_none());
    }
}
