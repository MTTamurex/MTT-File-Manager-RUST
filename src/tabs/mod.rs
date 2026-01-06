//! Tab management system for MTT File Manager
//! 
//! Each tab represents an independent file browser view with its own:
//! - Current path
//! - Navigation history
//! - Selected items
//! - Sort preferences

use std::path::PathBuf;
use std::sync::Arc;
use crate::domain::file_entry::FileEntry;

/// Represents a single browser tab
#[derive(Clone)]
pub struct TabState {
    /// Unique identifier for this tab
    pub id: usize,
    /// Current directory path
    pub path: String,
    /// Display title (folder name or "Este Computador")
    pub title: String,
    /// Navigation history (linear)
    pub navigation_history: Vec<String>,
    /// Current position in history
    pub history_index: usize,
    /// Whether this tab is showing "Este Computador" view
    pub is_computer_view: bool,
    /// Items in this tab's view
    pub items: Arc<Vec<FileEntry>>,
    /// Unfiltered items (for search)
    pub all_items: Vec<FileEntry>,
    /// Selected item index
    pub selected_item: Option<usize>,
    /// Selected file entry
    pub selected_file: Option<FileEntry>,
    /// Search query for this tab
    pub search_query: String,
    /// Whether to scroll to selected item on next frame
    pub scroll_to_selected: bool,
    /// Address bar input text
    pub path_input: String,
}

impl TabState {
    /// Create a new tab at "Este Computador"
    pub fn new_at_computer(id: usize) -> Self {
        Self {
            id,
            path: "Este Computador".to_string(),
            title: "Este Computador".to_string(),
            navigation_history: vec!["Este Computador".to_string()],
            history_index: 0,
            is_computer_view: true,
            items: Arc::new(Vec::new()),
            all_items: Vec::new(),
            selected_item: None,
            selected_file: None,
            search_query: String::new(),
            scroll_to_selected: false,
            path_input: "Este Computador".to_string(),
        }
    }
    
    /// Create a new tab at a specific path
    pub fn new_at_path(id: usize, path: &str) -> Self {
        let title = PathBuf::from(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string());
            
        Self {
            id,
            path: path.to_string(),
            title,
            navigation_history: vec![path.to_string()],
            history_index: 0,
            is_computer_view: false,
            items: Arc::new(Vec::new()),
            all_items: Vec::new(),
            selected_item: None,
            selected_file: None,
            search_query: String::new(),
            scroll_to_selected: false,
            path_input: path.to_string(),
        }
    }
    
    /// Navigate to a new path, pushing current to history
    pub fn navigate_to(&mut self, new_path: &str) {
        if new_path == self.path {
            return;
        }
        
        // Truncate future history if we were in the middle
        if self.history_index < self.navigation_history.len().saturating_sub(1) {
            self.navigation_history.truncate(self.history_index + 1);
        }
        
        // Add to history
        self.navigation_history.push(new_path.to_string());
        self.history_index = self.navigation_history.len() - 1;
        
        // Update current path
        self.path = new_path.to_string();
        self.path_input = new_path.to_string();
        self.is_computer_view = new_path == "Este Computador";
        
        // Update title
        if self.is_computer_view {
            self.title = "Este Computador".to_string();
        } else {
            self.title = PathBuf::from(new_path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| new_path.to_string());
        }
    }
    
    /// Go back in history
    pub fn go_back(&mut self) -> bool {
        if self.history_index > 0 {
            self.history_index -= 1;
            self.sync_from_history();
            true
        } else {
            false
        }
    }
    
    /// Go forward in history
    pub fn go_forward(&mut self) -> bool {
        if self.history_index + 1 < self.navigation_history.len() {
            self.history_index += 1;
            self.sync_from_history();
            true
        } else {
            false
        }
    }
    
    fn sync_from_history(&mut self) {
        if let Some(path) = self.navigation_history.get(self.history_index) {
            self.path = path.clone();
            self.path_input = path.clone();
            self.is_computer_view = path == "Este Computador";
            
            if self.is_computer_view {
                self.title = "Este Computador".to_string();
            } else {
                self.title = PathBuf::from(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone());
            }
        }
    }
    
    pub fn can_go_back(&self) -> bool {
        self.history_index > 0
    }
    
    pub fn can_go_forward(&self) -> bool {
        self.history_index + 1 < self.navigation_history.len()
    }
}

/// Manages all open tabs
pub struct TabManager {
    /// All open tabs
    pub tabs: Vec<TabState>,
    /// Index of the currently active tab
    pub active_tab: usize,
    /// Counter for generating unique tab IDs
    next_id: usize,
    /// Recently closed tabs (for reopening)
    closed_tabs: Vec<TabState>,
}

impl Default for TabManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TabManager {
    /// Create a new TabManager with one tab at "Este Computador"
    pub fn new() -> Self {
        let initial_tab = TabState::new_at_computer(0);
        Self {
            tabs: vec![initial_tab],
            active_tab: 0,
            next_id: 1,
            closed_tabs: Vec::new(),
        }
    }
    
    /// Get the currently active tab
    pub fn active(&self) -> &TabState {
        &self.tabs[self.active_tab]
    }
    
    /// Get mutable reference to active tab
    pub fn active_mut(&mut self) -> &mut TabState {
        &mut self.tabs[self.active_tab]
    }
    
    /// Add a new tab at "Este Computador" and switch to it
    pub fn new_tab(&mut self) {
        let tab = TabState::new_at_computer(self.next_id);
        self.next_id += 1;
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
    }
    
    /// Add a new tab at a specific path and switch to it
    pub fn new_tab_at(&mut self, path: &str) {
        let tab = TabState::new_at_path(self.next_id, path);
        self.next_id += 1;
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
    }
    
    /// Duplicate the current tab
    pub fn duplicate_tab(&mut self) {
        let current = self.active().clone();
        let mut new_tab = TabState::new_at_path(self.next_id, &current.path);
        new_tab.navigation_history = current.navigation_history.clone();
        new_tab.history_index = current.history_index;
        new_tab.is_computer_view = current.is_computer_view;
        new_tab.items = current.items.clone();
        new_tab.all_items = current.all_items.clone();
        new_tab.selected_item = current.selected_item;
        new_tab.selected_file = current.selected_file.clone();
        new_tab.search_query = current.search_query.clone();
        
        self.next_id += 1;
        
        // Insert after current tab
        let insert_pos = self.active_tab + 1;
        self.tabs.insert(insert_pos, new_tab);
        self.active_tab = insert_pos;
    }
    
    /// Close the tab at the given index
    /// Returns true if the app should close (no tabs left)
    pub fn close_tab(&mut self, index: usize) -> bool {
        if self.tabs.len() <= 1 {
            // Last tab - signal app should close
            return true;
        }
        
        // Save to closed tabs for potential reopening
        let closed = self.tabs.remove(index);
        self.closed_tabs.push(closed);
        
        // Keep max 10 closed tabs
        if self.closed_tabs.len() > 10 {
            self.closed_tabs.remove(0);
        }
        
        // Adjust active tab index
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        } else if self.active_tab > index {
            self.active_tab = self.active_tab.saturating_sub(1);
        }
        
        false
    }
    
    /// Close the currently active tab
    pub fn close_active_tab(&mut self) -> bool {
        self.close_tab(self.active_tab)
    }
    
    /// Switch to the tab at the given index
    pub fn switch_to(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active_tab = index;
        }
    }
    
    /// Switch to the next tab (wrapping around)
    pub fn next_tab(&mut self) {
        self.active_tab = (self.active_tab + 1) % self.tabs.len();
    }
    
    /// Switch to the previous tab (wrapping around)
    pub fn prev_tab(&mut self) {
        if self.active_tab == 0 {
            self.active_tab = self.tabs.len() - 1;
        } else {
            self.active_tab -= 1;
        }
    }
    
    /// Reopen the most recently closed tab
    pub fn reopen_closed_tab(&mut self) -> bool {
        if let Some(tab) = self.closed_tabs.pop() {
            let mut reopened = TabState::new_at_path(self.next_id, &tab.path);
            reopened.navigation_history = tab.navigation_history;
            reopened.history_index = tab.history_index;
            reopened.is_computer_view = tab.is_computer_view;
            reopened.items = tab.items;
            reopened.all_items = tab.all_items;
            reopened.selected_item = tab.selected_item;
            reopened.selected_file = tab.selected_file;
            reopened.search_query = tab.search_query;
            
            self.next_id += 1;
            
            // Insert after active tab
            let insert_pos = self.active_tab + 1;
            self.tabs.insert(insert_pos, reopened);
            self.active_tab = insert_pos;
            true
        } else {
            false
        }
    }
    
    /// Get number of open tabs
    pub fn count(&self) -> usize {
        self.tabs.len()
    }
}
