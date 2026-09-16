//! Navigation history management
//! Follows .cursorrules: single responsibility, < 300 lines

use std::collections::VecDeque;
use std::path::PathBuf;

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

    /// Removes deleted filesystem paths and their descendants from the
    /// timeline and recent-visit list. Returns whether the current history
    /// entry was removed so the caller can redirect the panel to a safe path.
    pub fn remove_paths_under(&mut self, deleted_paths: &[PathBuf]) -> bool {
        if deleted_paths.is_empty() || self.paths.is_empty() {
            return false;
        }

        let deleted_roots: Vec<String> = deleted_paths
            .iter()
            .map(|path| normalize_history_path(&path.to_string_lossy()))
            .filter(|path| !path.is_empty())
            .collect();
        if deleted_roots.is_empty() {
            return false;
        }

        let old_current_index = self.current_index.min(self.paths.len() - 1);
        let mut removed_before_current = 0usize;
        let mut current_was_removed = false;
        let mut retained = VecDeque::with_capacity(self.paths.len());

        for (index, path) in self.paths.drain(..).enumerate() {
            let is_removed = history_path_is_under_any(&path, &deleted_roots);
            if is_removed {
                if index < old_current_index {
                    removed_before_current += 1;
                } else if index == old_current_index {
                    current_was_removed = true;
                }
            } else {
                retained.push_back(path);
            }
        }

        self.paths = retained;
        self.current_index = if self.paths.is_empty() {
            0
        } else {
            old_current_index
                .saturating_sub(removed_before_current)
                .min(self.paths.len() - 1)
        };
        self.recent_paths
            .retain(|path| !history_path_is_under_any(path, &deleted_roots));

        current_was_removed
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

fn normalize_history_path(path: &str) -> String {
    let mut normalized = path.replace('/', "\\").to_ascii_lowercase();
    if let Some(stripped) = normalized.strip_prefix(r"\\?\") {
        normalized = stripped.to_string();
    }
    while normalized.len() > 3 && normalized.ends_with('\\') {
        normalized.pop();
    }
    normalized
}

fn history_path_is_under_any(path: &str, deleted_roots: &[String]) -> bool {
    let normalized = normalize_history_path(path);
    deleted_roots.iter().any(|root| {
        normalized == *root
            || (root.ends_with('\\') && normalized.starts_with(root))
            || (!root.ends_with('\\')
                && normalized.starts_with(root)
                && normalized.as_bytes().get(root.len()) == Some(&b'\\'))
    })
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

        assert_eq!(
            history.recent_paths(5),
            vec!["2".to_string(), "1".to_string()]
        );

        history.go_back();

        assert_eq!(history.current_path(), Some(&"2".to_string()));
        assert_eq!(
            history.recent_paths(5),
            vec!["3".to_string(), "1".to_string()]
        );
    }

    #[test]
    fn test_recent_paths_deduplicates_revisited_entries() {
        let mut history = NavigationHistory::new("1".to_string());
        history.navigate_to("2".to_string());
        history.navigate_to("3".to_string());
        history.navigate_to("2".to_string());

        assert_eq!(history.current_path(), Some(&"2".to_string()));
        assert_eq!(
            history.recent_paths(5),
            vec!["3".to_string(), "1".to_string()]
        );
    }

    #[test]
    fn remove_deleted_path_prunes_back_forward_and_recent_history() {
        let parent = r"C:\Users\user\Documents\Project";
        let deleted = r"C:\Users\user\Documents\Project\deleted";
        let mut history = NavigationHistory::new(parent.to_string());
        history.navigate_to(deleted.to_string());
        history.navigate_to(format!(r"{deleted}\child"));
        history.navigate_to(parent.to_string());

        assert!(!history.remove_paths_under(&[PathBuf::from(deleted)]));
        assert_eq!(history.paths.len(), 2);
        assert!(history.paths.iter().all(|path| !path.starts_with(deleted)));
        assert!(history
            .recent_paths(10)
            .iter()
            .all(|path| !path.starts_with(deleted)));
        assert_eq!(history.go_back(), Some(&parent.to_string()));
        assert_eq!(history.go_forward(), Some(&parent.to_string()));
    }

    #[test]
    fn remove_deleted_path_does_not_match_a_sibling_with_the_same_prefix() {
        let deleted = r"C:\Users\user\Documents\Project\deleted";
        let sibling = r"C:\Users\user\Documents\Project\deleted-copy";
        let mut history = NavigationHistory::new(deleted.to_string());
        history.navigate_to(sibling.to_string());

        assert!(!history.remove_paths_under(&[PathBuf::from(deleted)]));
        assert_eq!(history.current_path(), Some(&sibling.to_string()));
    }

    #[test]
    fn remove_deleted_current_path_reports_that_redirect_is_needed() {
        let deleted = r"C:\Users\user\Documents\Project\deleted";
        let mut history = NavigationHistory::new(deleted.to_string());

        assert!(history.remove_paths_under(&[PathBuf::from(deleted)]));
        assert!(history.paths.is_empty());
        assert!(!history.can_go_back());
        assert!(!history.can_go_forward());
    }
}
