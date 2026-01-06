//! Tab management system for MTT File Manager
//! 
//! Each tab represents an independent file browser view with its own:
//! - Current path
//! - Navigation history
//! - Selected items
//! - Sort preferences

use std::path::PathBuf;

/// Represents a single browser tab
#[derive(Clone)]
pub struct TabState {
    /// Unique identifier for this tab
    pub id: usize,
    /// Current directory path
    pub path: String,
    /// Display title (folder name or "Este Computador")
    pub title: String,
    /// Navigation history (back stack)
    pub history_back: Vec<String>,
    /// Navigation history (forward stack)  
    pub history_forward: Vec<String>,
    /// Whether this tab is showing "Este Computador" view
    pub is_computer_view: bool,
}

impl TabState {
    /// Create a new tab at "Este Computador"
    pub fn new_at_computer(id: usize) -> Self {
        Self {
            id,
            path: "Este Computador".to_string(),
            title: "Este Computador".to_string(),
            history_back: Vec::new(),
            history_forward: Vec::new(),
            is_computer_view: true,
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
            history_back: Vec::new(),
            history_forward: Vec::new(),
            is_computer_view: false,
        }
    }
    
    /// Navigate to a new path, pushing current to history
    pub fn navigate_to(&mut self, new_path: &str) {
        if new_path == self.path {
            return;
        }
        
        // Push current path to back history
        self.history_back.push(self.path.clone());
        
        // Clear forward history on new navigation
        self.history_forward.clear();
        
        // Update current path
        self.path = new_path.to_string();
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
        if let Some(prev_path) = self.history_back.pop() {
            self.history_forward.push(self.path.clone());
            self.path = prev_path.clone();
            self.is_computer_view = prev_path == "Este Computador";
            self.title = if self.is_computer_view {
                "Este Computador".to_string()
            } else {
                PathBuf::from(&prev_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| prev_path.clone())
            };
            true
        } else {
            false
        }
    }
    
    /// Go forward in history
    pub fn go_forward(&mut self) -> bool {
        if let Some(next_path) = self.history_forward.pop() {
            self.history_back.push(self.path.clone());
            self.path = next_path.clone();
            self.is_computer_view = next_path == "Este Computador";
            self.title = if self.is_computer_view {
                "Este Computador".to_string()
            } else {
                PathBuf::from(&next_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| next_path.clone())
            };
            true
        } else {
            false
        }
    }
    
    pub fn can_go_back(&self) -> bool {
        !self.history_back.is_empty()
    }
    
    pub fn can_go_forward(&self) -> bool {
        !self.history_forward.is_empty()
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
        new_tab.history_back = current.history_back.clone();
        new_tab.history_forward = current.history_forward.clone();
        new_tab.is_computer_view = current.is_computer_view;
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
            reopened.history_back = tab.history_back;
            reopened.history_forward = tab.history_forward;
            reopened.is_computer_view = tab.is_computer_view;
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
