//! Navigation functionality for the file manager.
//!
//! This module contains navigation methods like go_back, go_forward,
//! go_up_one_level, navigate_to, etc.

use crate::ui::app::ImageViewerApp;

impl ImageViewerApp {
    /// Navigates to a specific path.
    pub fn navigate_to(&mut self, path: &str) {
        let normalized_path = if path.ends_with('\\') || path.ends_with('/') {
            path.to_string()
        } else {
            format!("{}\\", path)
        };
        
        // Update history
        if self.history_index < self.history.len() {
            self.history.truncate(self.history_index + 1);
        }
        self.history.push(self.current_path.clone());
        self.history_index = self.history.len();
        
        // Update current path
        self.current_path = normalized_path.clone();
        self.is_computer_view = false;
        
        // Load folder
        self.load_folder();
    }
    
    /// Navigates to "Este Computador" view.
    pub fn navigate_to_computer(&mut self) {
        // Update history
        if self.history_index < self.history.len() {
            self.history.truncate(self.history_index + 1);
        }
        self.history.push(self.current_path.clone());
        self.history_index = self.history.len();
        
        // Set computer view
        self.current_path = "Este Computador".to_string();
        self.is_computer_view = true;
        
        // Clear items for computer view
        self.items.clear();
        self.all_items.clear();
        self.selected_item = None;
        self.selected_file = None;
        self.total_items = self.disks.len();
    }
    
    /// Goes back in navigation history.
    pub fn go_back(&mut self) {
        if self.can_go_back() {
            self.history_index -= 1;
            let prev_path = self.history[self.history_index].clone();
            
            if prev_path == "Este Computador" {
                self.current_path = prev_path;
                self.is_computer_view = true;
                self.items.clear();
                self.all_items.clear();
                self.selected_item = None;
                self.selected_file = None;
                self.total_items = self.disks.len();
            } else {
                self.current_path = prev_path;
                self.is_computer_view = false;
                self.load_folder();
            }
        }
    }
    
    /// Goes forward in navigation history.
    pub fn go_forward(&mut self) {
        if self.can_go_forward() {
            self.history_index += 1;
            let next_path = self.history[self.history_index].clone();
            
            if next_path == "Este Computador" {
                self.current_path = next_path;
                self.is_computer_view = true;
                self.items.clear();
                self.all_items.clear();
                self.selected_item = None;
                self.selected_file = None;
                self.total_items = self.disks.len();
            } else {
                self.current_path = next_path;
                self.is_computer_view = false;
                self.load_folder();
            }
        }
    }
    
    /// Goes up one level in the directory hierarchy.
    pub fn go_up_one_level(&mut self) {
        if self.is_computer_view {
            return; // Can't go up from computer view
        }
        
        let current_path = self.current_path.clone();
        let path = std::path::Path::new(&current_path);
        if let Some(parent) = path.parent() {
            if let Some(parent_str) = parent.to_str() {
                self.navigate_to(parent_str);
            }
        }
    }
    
    /// Checks if can go back in history.
    pub fn can_go_back(&self) -> bool {
        self.history_index > 0
    }
    
    /// Checks if can go forward in history.
    pub fn can_go_forward(&self) -> bool {
        self.history_index < self.history.len() - 1
    }
    
    /// Checks if can go up one level.
    pub fn can_go_up(&self) -> bool {
        !self.is_computer_view && std::path::Path::new(&self.current_path).parent().is_some()
    }
}
