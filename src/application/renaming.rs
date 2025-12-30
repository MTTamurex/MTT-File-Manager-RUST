//! File renaming state management
//! Follows .cursorrules: single responsibility, < 300 lines

/// Renaming state
#[derive(Clone, Debug)]
pub struct RenamingState {
    pub item_index: usize,
    pub new_name: String,
    pub focus_requested: bool,
}

impl RenamingState {
    /// Creates a new renaming state
    pub fn new(item_index: usize, current_name: String) -> Self {
        Self {
            item_index,
            new_name: current_name,
            focus_requested: true,
        }
    }
    
    /// Updates the new name
    pub fn update_name(&mut self, new_name: String) {
        self.new_name = new_name;
    }
    
    /// Marks focus as handled
    pub fn mark_focus_handled(&mut self) {
        self.focus_requested = false;
    }
    
    /// Checks if focus is requested
    pub fn focus_requested(&self) -> bool {
        self.focus_requested
    }
    
    /// Completes renaming and returns the state
    pub fn complete(self) -> (usize, String) {
        (self.item_index, self.new_name)
    }
}
