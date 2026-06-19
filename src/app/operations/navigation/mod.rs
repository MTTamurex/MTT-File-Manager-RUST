//! Navigation: navigate_to, go_back, go_forward, go_up
//!
//! This module handles history based navigation and switching to special views.

pub mod keyboard;
pub mod selection;

use crate::app::state::ImageViewerApp;
use crate::domain::special_paths::{
    is_virtual_path, tag_id_from_view_path, COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID,
};
use std::path::{Path, PathBuf};

impl ImageViewerApp {
    fn remember_current_folder_timestamp_hints(&mut self) {
        if let Some((path, modified)) = self.current_folder_modified_hint.as_ref() {
            if *modified > 0 {
                self.folder_modified_hints.put(path.clone(), *modified);
            }
        }
        if let Some((path, created)) = self.current_folder_created_hint.as_ref() {
            if *created > 0 {
                self.folder_created_hints.put(path.clone(), *created);
            }
        }
    }

    fn resolve_destination_folder_modified_hint(
        &self,
        destination_path: &Path,
    ) -> Option<(PathBuf, u64)> {
        self.items
            .iter()
            .find(|item| item.is_dir && item.path == destination_path && item.modified > 0)
            .map(|item| (item.path.clone(), item.modified))
            .or_else(|| {
                self.selected_file.as_ref().and_then(|selected| {
                    if selected.is_dir && selected.path == destination_path && selected.modified > 0
                    {
                        Some((selected.path.clone(), selected.modified))
                    } else {
                        None
                    }
                })
            })
            .or_else(|| {
                self.folder_modified_hints
                    .peek(destination_path)
                    .copied()
                    .filter(|modified| *modified > 0)
                    .map(|modified| (destination_path.to_path_buf(), modified))
            })
    }

    fn resolve_destination_folder_created_hint(
        &self,
        destination_path: &Path,
    ) -> Option<(PathBuf, u64)> {
        self.items
            .iter()
            .find(|item| item.is_dir && item.path == destination_path)
            .and_then(|item| item.created.map(|created| (item.path.clone(), created)))
            .filter(|(_, created)| *created > 0)
            .or_else(|| {
                self.selected_file.as_ref().and_then(|selected| {
                    if selected.is_dir && selected.path == destination_path {
                        selected
                            .created
                            .map(|created| (selected.path.clone(), created))
                    } else {
                        None
                    }
                })
            })
            .filter(|(_, created)| *created > 0)
            .or_else(|| {
                self.folder_created_hints
                    .peek(destination_path)
                    .copied()
                    .filter(|created| *created > 0)
                    .map(|created| (destination_path.to_path_buf(), created))
            })
    }

    /// Spawns a background thread to read folder metadata when no cached timestamp
    /// exists (e.g. first visit to a Quick Access or Cloud Drive folder).
    /// The result is sent back via `folder_meta_resolve_rx` and applied in
    /// `process_incoming_messages` without blocking the UI thread.
    fn spawn_folder_meta_resolve_if_needed(&self, dest_path: &Path, path_str: &str) {
        if self.current_folder_modified_hint.is_some()
            || is_virtual_path(path_str)
            || crate::infrastructure::io_priority::is_network_or_virtual(dest_path)
        {
            return;
        }
        let tx = self.folder_meta_resolve_tx.clone();
        let dest = dest_path.to_path_buf();
        let current_path = path_str.to_string();
        std::thread::Builder::new()
            .name("folder-meta-resolve".into())
            .spawn(move || {
                if let Ok(meta) = std::fs::metadata(&dest) {
                    let modified = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let created = meta
                        .created()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs());
                    let _ = tx.send((PathBuf::from(current_path), modified, created));
                }
            })
            .ok();
    }

    pub fn navigate_to(&mut self, path: &str) {
        // Normalize drive root paths: ensure "Z:" always becomes "Z:\"
        // This fixes the PathBuf::join bug of not adding a backslash
        let normalized_path = if path.len() >= 2 && path.chars().nth(1) == Some(':') {
            // It's a Windows path with a drive letter
            if path.len() == 2 {
                // Just "Z:" -> "Z:\"
                format!("{}\\", path)
            } else if path.chars().nth(2) != Some('\\') {
                // "Z:folder" -> "Z:\folder" (fix malformed path)
                format!("{}\\{}", &path[0..2], &path[2..])
            } else {
                path.to_string()
            }
        } else {
            path.to_string()
        };

        // If we're already at this path, do nothing
        if self.navigation_state.current_path == normalized_path {
            return;
        }

        if let Some(tag_id) = tag_id_from_view_path(&normalized_path) {
            self.set_tag_filter(Some(tag_id));
            return;
        }

        if self.active_tag_filter.take().is_some() {
            self.save_preferences();
        }

        self.remember_current_folder_timestamp_hints();

        // Keep the folder "Data modificada" visible in preview panel after entering a folder.
        // Reuse the timestamp already present in current list/selection instead of doing
        // blocking filesystem metadata calls in the render loop.
        let destination_path = PathBuf::from(&normalized_path);
        self.current_folder_modified_hint =
            self.resolve_destination_folder_modified_hint(&destination_path);
        self.current_folder_created_hint =
            self.resolve_destination_folder_created_hint(&destination_path);

        // Fallback for pinned sidebar shortcuts pointing to never-visited folders:
        // resolve_destination_folder_modified_hint only knows about folders already seen
        // in the current session. When clicking a pinned shortcut for the first time,
        // no hint exists → modified = 0 → "Desconhecido" in the preview panel.
        //
        // Spawn a background thread to read metadata without blocking the UI thread.
        // On a sleeping HDD this avoids a 500-2000ms stall (spin-up).
        self.spawn_folder_meta_resolve_if_needed(&destination_path, &normalized_path);

        // Clear loaded_path to allow reload if navigating to same path (for consistency)
        self.loaded_path.clear();

        // Add new path to history
        self.navigation_state
            .navigation
            .navigate_to(normalized_path.clone());

        self.navigation_state.current_path = normalized_path.clone();
        self.navigation_state.path_input = normalized_path.clone();
        self.navigation_state.is_computer_view = false;
        self.navigation_state.is_recycle_bin_view = false; // Reset when navigating to any folder

        // Restore normal folder sort mode
        self.sort_mode = self.sort_mode_normal;

        self.reset_selection_and_search();

        // Apply folder lock if this folder has locked preferences
        self.apply_folder_lock_if_present();

        // UPDATE THE WATCHER
        self.watch_current_folder();

        // Clear old items immediately for path-change navigation.
        // Prevents stale items from the previous folder flashing on screen
        // while the new folder loads (especially noticeable for archives
        // like ZIP/RAR which take longer to enumerate via Shell API).
        // Watcher-triggered reloads (same path) don't go through navigate_to()
        // so they still benefit from stale-while-revalidate in load_folder.
        self.items = std::sync::Arc::new(Vec::new());
        self.all_items_mut().clear();

        // Cancel pending batch folder-size calculations for the old folder.
        self.folder_size_state.cancel_batch();

        // Discard pending mtime rechecks for the old folder's subfolders.
        self.pending_folder_mtime_recheck.clear();

        // SYNC TAB STATE after clearing stale lists to avoid heavy cloning on navigation.
        self.sync_to_tab();

        self.load_folder(false);
    }

    pub fn go_back(&mut self) {
        if let Some(path) = self.navigation_state.navigation.go_back().cloned() {
            self.remember_current_folder_timestamp_hints();

            // Cancel pending batch folder-size calculations for the old folder.
            self.folder_size_state.cancel_batch();

            // Save current path before going back (to invalidate the preview)
            let previous_path = std::path::PathBuf::from(&self.navigation_state.current_path);

            if path == COMPUTER_VIEW_ID {
                if self.active_tag_filter.take().is_some() {
                    self.save_preferences();
                }
                // Invalidate preview of the folder we were in
                self.cache_manager.invalidate_folder_preview(&previous_path);

                // SYNC TAB STATE
                self.sync_to_tab();

                self.reset_selection_and_search();
                self.setup_computer_view();
            } else if path == RECYCLE_BIN_VIEW_ID {
                if self.active_tag_filter.take().is_some() {
                    self.save_preferences();
                }
                // Invalidate preview of the folder we were in
                self.cache_manager.invalidate_folder_preview(&previous_path);

                self.reset_selection_and_search();
                self.setup_recycle_bin_view();
            } else if let Some(tag_id) = tag_id_from_view_path(&path) {
                self.reset_selection_and_search();
                self.setup_tag_view(tag_id);
                self.sync_to_tab();
            } else {
                if self.active_tag_filter.take().is_some() {
                    self.save_preferences();
                }
                let new_path = std::path::PathBuf::from(&path);
                self.current_folder_modified_hint =
                    self.resolve_destination_folder_modified_hint(&new_path);
                self.current_folder_created_hint =
                    self.resolve_destination_folder_created_hint(&new_path);
                self.spawn_folder_meta_resolve_if_needed(&new_path, &path);

                // If we were in a subfolder of the destination, invalidate that subfolder's preview
                if previous_path.starts_with(&new_path) && previous_path != new_path {
                    self.cache_manager.invalidate_folder_preview(&previous_path);
                }

                self.navigation_state.current_path = path.clone();
                self.loaded_path.clear(); // Clear to allow reload
                self.sync_to_tab();
                self.navigation_state.path_input = self.navigation_state.current_path.clone();
                self.navigation_state.is_computer_view = false;
                self.navigation_state.is_recycle_bin_view = false;

                // Restore normal folder sort mode
                self.sort_mode = self.sort_mode_normal;

                self.reset_selection_and_search();
                self.apply_folder_lock_if_present();
                self.watch_current_folder(); // Update the watcher

                // Clear stale items (see navigate_to comment)
                self.items = std::sync::Arc::new(Vec::new());
                self.all_items_mut().clear();

                // SYNC TAB STATE after clearing stale lists to avoid heavy cloning on navigation.
                self.sync_to_tab();

                self.load_folder(false);
            }
        }
    }

    /// Moves forward in history
    pub fn go_forward(&mut self) {
        if let Some(path) = self.navigation_state.navigation.go_forward().cloned() {
            self.remember_current_folder_timestamp_hints();

            // Cancel pending batch folder-size calculations for the old folder.
            self.folder_size_state.cancel_batch();

            // Save current path before going forward (to invalidate the preview)
            let previous_path = std::path::PathBuf::from(&self.navigation_state.current_path);

            if path == COMPUTER_VIEW_ID {
                if self.active_tag_filter.take().is_some() {
                    self.save_preferences();
                }
                // Invalidate preview of the folder we were in
                self.cache_manager.invalidate_folder_preview(&previous_path);

                // SYNC TAB STATE
                self.sync_to_tab();

                self.reset_selection_and_search();
                self.setup_computer_view();
            } else if path == RECYCLE_BIN_VIEW_ID {
                if self.active_tag_filter.take().is_some() {
                    self.save_preferences();
                }
                // Invalidate preview of the folder we were in
                self.cache_manager.invalidate_folder_preview(&previous_path);

                self.reset_selection_and_search();
                self.setup_recycle_bin_view();
            } else if let Some(tag_id) = tag_id_from_view_path(&path) {
                self.reset_selection_and_search();
                self.setup_tag_view(tag_id);
                self.sync_to_tab();
            } else {
                if self.active_tag_filter.take().is_some() {
                    self.save_preferences();
                }
                let new_path = std::path::PathBuf::from(&path);
                self.current_folder_modified_hint =
                    self.resolve_destination_folder_modified_hint(&new_path);
                self.current_folder_created_hint =
                    self.resolve_destination_folder_created_hint(&new_path);
                self.spawn_folder_meta_resolve_if_needed(&new_path, &path);

                // If we were in a subfolder of the destination, invalidate that subfolder's preview
                if previous_path.starts_with(&new_path) && previous_path != new_path {
                    self.cache_manager.invalidate_folder_preview(&previous_path);
                }

                self.navigation_state.current_path = path.clone();
                self.loaded_path.clear(); // Clear to allow reload
                self.sync_to_tab();
                self.navigation_state.path_input = self.navigation_state.current_path.clone();
                self.navigation_state.is_computer_view = false;
                self.navigation_state.is_recycle_bin_view = false;

                // Restore normal folder sort mode
                self.sort_mode = self.sort_mode_normal;

                self.reset_selection_and_search();
                self.apply_folder_lock_if_present();
                self.watch_current_folder();

                // Clear stale items (see navigate_to comment)
                self.items = std::sync::Arc::new(Vec::new());
                self.all_items_mut().clear();

                // SYNC TAB STATE after clearing stale lists to avoid heavy cloning on navigation.
                self.sync_to_tab();

                self.load_folder(false);
            }
        }
    }

    /// Navigates to the "This PC" view (adding to history)
    pub fn navigate_to_computer(&mut self) {
        if self.navigation_state.is_computer_view {
            return;
        }

        if self.active_tag_filter.take().is_some() {
            self.save_preferences();
        }

        // Cancel pending batch folder-size calculations.
        self.folder_size_state.cancel_batch();

        self.navigation_state
            .navigation
            .navigate_to(COMPUTER_VIEW_ID.to_string());
        // self.sync_to_tab(); // setup_computer_view calls sync_from_tab?? no, we sync afterward

        self.reset_selection_and_search();
        self.watch_current_folder();
        self.setup_computer_view();
        self.sync_to_tab();
    }

    pub fn navigate_to_recycle_bin(&mut self) {
        if self.navigation_state.is_recycle_bin_view {
            return;
        }

        if self.active_tag_filter.take().is_some() {
            self.save_preferences();
        }

        // Cancel pending batch folder-size calculations.
        self.folder_size_state.cancel_batch();

        self.navigation_state
            .navigation
            .navigate_to(RECYCLE_BIN_VIEW_ID.to_string());
        self.reset_selection_and_search();
        // NOTE: Do NOT call watch_current_folder() here.
        // Recycle bin is a virtual view, not a real filesystem path — notify would
        // fail with "Input watch path is neither a file nor a directory".
        self.setup_recycle_bin_view();
        self.sync_to_tab();
    }

    pub fn go_up_one_level(&mut self) {
        if self.navigation_state.is_computer_view {
            // Already at the top
            return;
        }

        if is_virtual_path(&self.navigation_state.current_path) {
            self.navigate_to_computer();
            return;
        }

        // If we're at the root of a drive (C:\, D:\), going up navigates to "This PC"
        let parent = std::path::Path::new(&self.navigation_state.current_path)
            .parent()
            .map(std::path::Path::to_path_buf);
        if parent.is_none() {
            self.navigate_to_computer();
            return;
        }

        if let Some(parent_path) = parent {
            if parent_path.as_os_str().is_empty() {
                self.navigate_to_computer();
            } else {
                let target = parent_path.to_string_lossy();
                self.navigate_to(target.as_ref());
            }
        } else {
            self.navigate_to_computer();
        }
    }

    /// Can go back in history?
    pub fn can_go_back(&self) -> bool {
        self.navigation_state.navigation.can_go_back()
    }

    /// Can go forward in history?
    pub fn can_go_forward(&self) -> bool {
        self.navigation_state.navigation.can_go_forward()
    }

    /// Navigates upward from the given path until a valid (existing) ancestor
    /// folder is found. If no ancestor exists (e.g. drive removed), navigates
    /// to "Este Computador".
    ///
    /// Used when the current folder is deleted externally.
    pub fn navigate_to_nearest_valid_ancestor(&mut self) {
        let current = PathBuf::from(&self.navigation_state.current_path);
        log::warn!(
            "[NAV] Current folder no longer exists: {:?}  — searching for valid ancestor",
            current
        );

        // FIX: Avoid blocking is_dir() calls on the UI thread.
        // GetFileAttributesW can block indefinitely on network/cloud/USB drives.
        // Instead, navigate directly to the parent directory. If the parent
        // doesn't exist either, the loading pipeline will detect the error
        // and we'll handle it via the next watcher event / consistency probe.
        // For root drives (e.g. "E:\"), go straight to computer view.
        if let Some(parent) = current.as_path().parent() {
            if !parent.as_os_str().is_empty() {
                log::info!(
                    "[NAV] Navigating to parent: {:?} (no blocking I/O check)",
                    parent
                );
                let target = parent.to_string_lossy();
                self.navigate_to(target.as_ref());
                return;
            }
        }

        // No valid ancestor (root of drive or empty) → go to computer view
        log::warn!("[NAV] No parent available — redirecting to Este Computador");
        self.navigate_to_computer();
    }
}

// Re-export commonly used types from submodules
pub use keyboard::*;
pub use selection::*;
