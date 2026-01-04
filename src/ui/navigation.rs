//! Navigation functionality for the file manager.
//!
//! This module contains navigation methods like go_back, go_forward,
//! go_up_one_level, navigate_to, etc.

use std::path::Path;
use std::sync::Arc;

// Note: ImageViewerApp is defined in main.rs, not in a module.
// These methods will be implemented in main.rs using this module.

/// Navigation operations trait that can be implemented by the main app
pub trait NavigationOperations {
    fn navigate_to(&mut self, path: &str);
    fn go_back(&mut self);
    fn go_forward(&mut self);
    fn navigate_to_computer(&mut self);
    fn go_up_one_level(&mut self);
    fn can_go_back(&self) -> bool;
    fn can_go_forward(&self) -> bool;
    fn can_go_up(&self) -> bool;
}

/// Helper functions for navigation logic
pub mod helpers {
    use super::*;

    /// Navigates to a specific path (implementation logic)
    pub fn navigate_to_impl(
        current_path: &mut String,
        path_input: &mut String,
        navigation_history: &mut Vec<String>,
        history_index: &mut usize,
        context_menu_target_path: &mut Option<std::path::PathBuf>,
        watch_current_folder: &mut dyn FnMut(),
        load_folder: &mut dyn FnMut(),
        path: &str,
    ) {
        // If already at this path, do nothing
        if *current_path == path {
            return;
        }

        // Cut "future" history (if we went back and navigated elsewhere)
        if *history_index < navigation_history.len().saturating_sub(1) {
            navigation_history.truncate(*history_index + 1);
        }

        // Add new path to history
        navigation_history.push(path.to_string());
        *history_index = navigation_history.len() - 1;

        *current_path = path.to_string();
        *path_input = path.to_string();

        // Clear context_menu.target_path to ensure sync with current folder
        *context_menu_target_path = None;

        // UPDATE WATCHER
        watch_current_folder();

        load_folder();
    }

    /// Goes back in navigation history (without adding to history)
    pub fn go_back_impl(
        current_path: &mut String,
        path_input: &mut String,
        navigation_history: &Vec<String>,
        history_index: &mut usize,
        watch_current_folder: &mut dyn FnMut(),
        load_folder: &mut dyn FnMut(),
    ) -> bool {
        if *history_index > 0 {
            *history_index -= 1;
            *current_path = navigation_history[*history_index].clone();
            *path_input = current_path.clone();
            watch_current_folder();
            load_folder();
            true
        } else {
            false
        }
    }

    /// Goes forward in navigation history
    pub fn go_forward_impl(
        current_path: &mut String,
        path_input: &mut String,
        navigation_history: &Vec<String>,
        history_index: &mut usize,
        watch_current_folder: &mut dyn FnMut(),
        load_folder: &mut dyn FnMut(),
    ) -> bool {
        if *history_index < navigation_history.len().saturating_sub(1) {
            *history_index += 1;
            *current_path = navigation_history[*history_index].clone();
            *path_input = current_path.clone();
            watch_current_folder();
            load_folder();
            true
        } else {
            false
        }
    }

    /// Navigates to "Este Computador" view
    pub fn navigate_to_computer_impl(
        current_path: &mut String,
        path_input: &mut String,
        is_computer_view: &mut bool,
        navigation_history: &mut Vec<String>,
        history_index: &mut usize,
        items: &mut Arc<Vec<crate::domain::file_entry::FileEntry>>,
        all_items: &mut Vec<crate::domain::file_entry::FileEntry>,
        selected_item: &mut Option<usize>,
        selected_file: &mut Option<crate::domain::file_entry::FileEntry>,
        total_items: &mut usize,
        disks_len: usize,
    ) {
        // Update history
        if *history_index < navigation_history.len() {
            navigation_history.truncate(*history_index + 1);
        }
        navigation_history.push(current_path.clone());
        *history_index = navigation_history.len();

        // Set computer view
        *current_path = "Este Computador".to_string();
        *is_computer_view = true;
        *path_input = "Este Computador".to_string();

        // Clear items for computer view
        *items = Arc::new(Vec::new());
        all_items.clear();
        *selected_item = None;
        *selected_file = None;
        *total_items = disks_len;
    }

    /// Goes up one level (adds to history)
    pub fn go_up_one_level_impl(current_path: &str, navigate_to: &mut dyn FnMut(&str)) -> bool {
        if let Some(parent) = Path::new(current_path).parent() {
            let parent_str = parent.to_string_lossy().to_string();
            if !parent_str.is_empty() {
                navigate_to(&parent_str);
                return true;
            }
        }
        false
    }

    /// Can go back in history?
    pub fn can_go_back_impl(history_index: usize) -> bool {
        history_index > 0
    }

    /// Can go forward in history?
    pub fn can_go_forward_impl(history_index: usize, navigation_history_len: usize) -> bool {
        history_index < navigation_history_len.saturating_sub(1)
    }

    /// Can go up one level?
    pub fn can_go_up_impl(is_computer_view: bool, current_path: &str) -> bool {
        !is_computer_view && Path::new(current_path).parent().is_some()
    }
}
