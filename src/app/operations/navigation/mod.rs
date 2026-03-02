//! Navigation: navigate_to, go_back, go_forward, go_up
//!
//! This module handles history based navigation and switching to special views.

pub mod keyboard;
pub mod selection;

use crate::app::state::ImageViewerApp;
use std::path::{Path, PathBuf};

impl ImageViewerApp {
    fn remember_current_folder_modified_hint(&mut self) {
        if let Some((path, modified)) = self.current_folder_modified_hint.as_ref() {
            if *modified > 0 {
                self.folder_modified_hints.insert(path.clone(), *modified);
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
                    .get(destination_path)
                    .copied()
                    .filter(|modified| *modified > 0)
                    .map(|modified| (destination_path.to_path_buf(), modified))
            })
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

        self.remember_current_folder_modified_hint();

        // Keep the folder "Data modificada" visible in preview panel after entering a folder.
        // Reuse the timestamp already present in current list/selection instead of doing
        // blocking filesystem metadata calls in the render loop.
        let destination_path = PathBuf::from(&normalized_path);
        self.current_folder_modified_hint =
            self.resolve_destination_folder_modified_hint(&destination_path);

        // Fallback for pinned sidebar shortcuts pointing to never-visited folders:
        // resolve_destination_folder_modified_hint only knows about folders already seen
        // in the current session. When clicking a pinned shortcut for the first time,
        // no hint exists → modified = 0 → "Desconhecido" in the preview panel.
        //
        // We intentionally do NOT call std::fs::metadata() here because it blocks
        // the UI thread. On a sleeping HDD this can stall for 500-2000ms (spin-up).
        // The timestamp will resolve naturally when the folder items finish loading.

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
        self.all_items.clear();

        // SYNC TAB STATE after clearing stale lists to avoid heavy cloning on navigation.
        self.sync_to_tab();

        self.load_folder(false);
    }

    pub fn go_back(&mut self) {
        if let Some(path) = self.navigation_state.navigation.go_back().cloned() {
            self.remember_current_folder_modified_hint();

            // Save current path before going back (to invalidate the preview)
            let previous_path = std::path::PathBuf::from(&self.navigation_state.current_path);

            if path == "Este Computador" {
                // Invalidate preview of the folder we were in
                self.cache_manager.invalidate_folder_preview(&previous_path);

                // SYNC TAB STATE
                self.sync_to_tab();

                self.reset_selection_and_search();
                self.setup_computer_view();
            } else if path == "Lixeira" {
                // Invalidate preview of the folder we were in
                self.cache_manager.invalidate_folder_preview(&previous_path);

                self.reset_selection_and_search();
                self.setup_recycle_bin_view();
            } else {
                let new_path = std::path::PathBuf::from(&path);
                self.current_folder_modified_hint =
                    self.resolve_destination_folder_modified_hint(&new_path);

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
                self.all_items.clear();

                // SYNC TAB STATE after clearing stale lists to avoid heavy cloning on navigation.
                self.sync_to_tab();

                self.load_folder(false);
            }
        }
    }

    /// Moves forward in history
    pub fn go_forward(&mut self) {
        if let Some(path) = self.navigation_state.navigation.go_forward().cloned() {
            self.remember_current_folder_modified_hint();

            // Save current path before going forward (to invalidate the preview)
            let previous_path = std::path::PathBuf::from(&self.navigation_state.current_path);

            if path == "Este Computador" {
                // Invalidate preview of the folder we were in
                self.cache_manager.invalidate_folder_preview(&previous_path);

                // SYNC TAB STATE
                self.sync_to_tab();

                self.reset_selection_and_search();
                self.setup_computer_view();
            } else if path == "Lixeira" {
                // Invalidate preview of the folder we were in
                self.cache_manager.invalidate_folder_preview(&previous_path);

                self.reset_selection_and_search();
                self.setup_recycle_bin_view();
            } else {
                let new_path = std::path::PathBuf::from(&path);
                self.current_folder_modified_hint =
                    self.resolve_destination_folder_modified_hint(&new_path);

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
                self.all_items.clear();

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

        self.navigation_state
            .navigation
            .navigate_to("Este Computador".to_string());
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

        self.navigation_state
            .navigation
            .navigate_to("Lixeira".to_string());
        self.reset_selection_and_search();
        // NOTE: Do NOT call watch_current_folder() here.
        // "Lixeira" is a virtual view, not a real filesystem path — notify would
        // fail with "Input watch path is neither a file nor a directory".
        self.setup_recycle_bin_view();
        self.sync_to_tab();
    }

    pub fn go_up_one_level(&mut self) {
        if self.navigation_state.is_computer_view {
            // Already at the top
            return;
        }

        // If we're at the root of a drive (C:\, D:\), going up navigates to "This PC"
        let parent = std::path::Path::new(&self.navigation_state.current_path).parent();
        if parent.is_none() {
            self.navigate_to_computer();
            return;
        }

        if let Some(parent_path) = parent {
            if parent_path.as_os_str().is_empty() {
                self.navigate_to_computer();
            } else {
                self.navigate_to(parent_path.to_string_lossy().to_string().as_str());
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

        let mut candidate = current.as_path().parent();
        while let Some(parent) = candidate {
            if parent.as_os_str().is_empty() {
                break;
            }
            if parent.is_dir() {
                log::info!("[NAV] Navigating to nearest valid ancestor: {:?}", parent);
                let target = parent.to_string_lossy().to_string();
                self.navigate_to(&target);
                return;
            }
            candidate = parent.parent();
        }

        // No valid ancestor on disk → go to computer view
        log::warn!("[NAV] No valid ancestor found — redirecting to Este Computador");
        self.navigate_to_computer();
    }
}

// Re-export commonly used types from submodules
pub use keyboard::*;
pub use selection::*;
