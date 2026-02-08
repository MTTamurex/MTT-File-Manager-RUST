//! Navigation history management
//! Follows .cursorrules: single responsibility, < 300 lines

use std::collections::VecDeque;

/// Navigation history with linear timeline
#[derive(Clone, Debug)]
pub struct NavigationHistory {
    pub paths: VecDeque<String>,
    pub current_index: usize,
}

impl NavigationHistory {
    /// Creates a new navigation history starting at the given path
    pub fn new(initial_path: String) -> Self {
        let mut paths = VecDeque::new();
        paths.push_back(initial_path.clone());

        Self {
            paths,
            current_index: 0,
        }
    }

    /// Navigates to a new path, cutting off future history
    pub fn navigate_to(&mut self, path: String) {
        // Cut off future history if we're not at the end
        if self.current_index < self.paths.len().saturating_sub(1) {
            self.paths.truncate(self.current_index + 1);
        }

        self.paths.push_back(path);
        self.current_index = self.paths.len() - 1;
    }

    /// Goes back in history
    pub fn go_back(&mut self) -> Option<&String> {
        if self.current_index > 0 {
            self.current_index -= 1;
            self.paths.get(self.current_index)
        } else {
            None
        }
    }

    /// Goes forward in history
    pub fn go_forward(&mut self) -> Option<&String> {
        if self.current_index < self.paths.len().saturating_sub(1) {
            self.current_index += 1;
            self.paths.get(self.current_index)
        } else {
            None
        }
    }

    /// Gets current path
    pub fn current_path(&self) -> Option<&String> {
        self.paths.get(self.current_index)
    }

    /// Checks if can go back
    pub fn can_go_back(&self) -> bool {
        self.current_index > 0
    }

    /// Checks if can go forward
    pub fn can_go_forward(&self) -> bool {
        self.current_index < self.paths.len().saturating_sub(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_navigation_history_truncation() {
        let mut history = NavigationHistory::new("1".to_string());
        history.navigate_to("2".to_string());
        history.navigate_to("3".to_string());

        history.go_back(); // Now at "2"
        assert_eq!(history.current_path(), Some(&"2".to_string()));

        // Navigate to "4" from "2" should truncate "3"
        history.navigate_to("4".to_string());
        assert_eq!(history.current_index, 2);
        assert_eq!(history.paths.len(), 3);
        assert_eq!(history.paths[2], "4");
        assert!(!history.can_go_forward());
    }

    #[test]
    fn test_navigation_history_edge_cases() {
        let mut history = NavigationHistory::new("1".to_string());
        assert_eq!(history.go_back(), None);
        assert_eq!(history.go_forward(), None);

        history.navigate_to("2".to_string());
        history.go_back();
        assert_eq!(history.current_index, 0);

        history.go_forward();
        assert_eq!(history.current_index, 1);
    }
}
