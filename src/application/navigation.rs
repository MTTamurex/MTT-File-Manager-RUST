//! Navigation history management
//! Follows .cursorrules: single responsibility, < 300 lines

use std::collections::VecDeque;

/// Cap on navigation history entries — prevents unbounded growth in long sessions (M-22)
const MAX_HISTORY: usize = 500;

/// Cap on MRU entries shown by the address bar dropdown.
const MAX_RECENT_VISITS: usize = 32;

/// Navigation history with linear timeline
#[derive(Clone, Debug)]
pub struct NavigationHistory {
    pub paths: VecDeque<String>,
    pub current_index: usize,
    recent_paths: VecDeque<String>,
}

impl NavigationHistory {
    /// Creates a new navigation history starting at the given path
    pub fn new(initial_path: String) -> Self {
        let mut paths = VecDeque::new();
        let mut recent_paths = VecDeque::new();
        paths.push_back(initial_path.clone());
        recent_paths.push_front(initial_path);

        Self {
            paths,
            current_index: 0,
            recent_paths,
        }
    }

    fn record_visit(&mut self, path: &str) {
        if let Some(existing_index) = self.recent_paths.iter().position(|entry| entry == path) {
            self.recent_paths.remove(existing_index);
        }

        self.recent_paths.push_front(path.to_string());

        while self.recent_paths.len() > MAX_RECENT_VISITS {
            self.recent_paths.pop_back();
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
        let current_path = self.paths[self.current_index].clone();
        self.record_visit(&current_path);

        // M-22: cap history to prevent unbounded growth in long sessions
        while self.paths.len() > MAX_HISTORY {
            self.paths.pop_front();
            self.current_index = self.current_index.saturating_sub(1);
        }
    }

    /// Goes back in history
    pub fn go_back(&mut self) -> Option<&String> {
        if self.current_index > 0 {
            self.current_index -= 1;
            if let Some(path) = self.paths.get(self.current_index).cloned() {
                self.record_visit(&path);
            }
            self.paths.get(self.current_index)
        } else {
            None
        }
    }

    /// Goes forward in history
    pub fn go_forward(&mut self) -> Option<&String> {
        if self.current_index < self.paths.len().saturating_sub(1) {
            self.current_index += 1;
            if let Some(path) = self.paths.get(self.current_index).cloned() {
                self.record_visit(&path);
            }
            self.paths.get(self.current_index)
        } else {
            None
        }
    }

    /// Gets current path
    pub fn current_path(&self) -> Option<&String> {
        self.paths.get(self.current_index)
    }

    /// Returns most recently visited paths, excluding the current path.
    pub fn recent_paths(&self, limit: usize) -> Vec<String> {
        let current_path = self.current_path().map(String::as_str);

        self.recent_paths
            .iter()
            .filter(|path| Some(path.as_str()) != current_path)
            .take(limit)
            .cloned()
            .collect()
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

    #[test]
    fn test_recent_paths_tracks_actual_visit_order() {
        let mut history = NavigationHistory::new("1".to_string());
        history.navigate_to("2".to_string());
        history.navigate_to("3".to_string());

        assert_eq!(history.recent_paths(5), vec!["2".to_string(), "1".to_string()]);

        history.go_back();

        assert_eq!(history.current_path(), Some(&"2".to_string()));
        assert_eq!(history.recent_paths(5), vec!["3".to_string(), "1".to_string()]);
    }

    #[test]
    fn test_recent_paths_deduplicates_revisited_entries() {
        let mut history = NavigationHistory::new("1".to_string());
        history.navigate_to("2".to_string());
        history.navigate_to("3".to_string());
        history.navigate_to("2".to_string());

        assert_eq!(history.current_path(), Some(&"2".to_string()));
        assert_eq!(history.recent_paths(5), vec!["3".to_string(), "1".to_string()]);
    }
}
