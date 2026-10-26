use super::{TabManager, TabState};

impl TabManager {
    /// Add a tab without disturbing the active tab or its live state.
    pub(crate) fn new_background_tab_at(&mut self, path: &str) -> Option<usize> {
        if !self.can_add_tab() {
            return None;
        }
        let mut tab = TabState::new_at_path(self.next_id, path);
        let current = self.active();
        tab.show_left_sidebar = current.show_left_sidebar;
        tab.show_preview_panel = current.show_preview_panel;
        tab.collapse_tags = current.collapse_tags;
        tab.group_mode = current.group_mode;
        tab.group_descending = current.group_descending;
        self.next_id += 1;
        self.tabs.push(tab);
        Some(self.tabs.len() - 1)
    }

    /// Add a tab and switch to it, preserving the existing explicit API.
    pub fn new_tab_at(&mut self, path: &str) -> bool {
        let Some(index) = self.new_background_tab_at(path) else {
            return false;
        };
        self.active_tab = index;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tabs::MAX_TABS;

    #[test]
    fn background_tabs_preserve_selection_scroll_and_active_index() {
        let mut manager = TabManager::new_at_path("C:\\");
        manager
            .active_mut()
            .multi_selection
            .insert("file.txt".into());
        manager.active_mut().scroll_offset_y = 120.0;
        let source_id = manager.active().id;
        let first = manager.new_background_tab_at("D:\\").unwrap();
        manager.new_background_tab_at("E:\\").unwrap();
        assert_eq!(manager.active().id, source_id);
        assert!(manager
            .active()
            .multi_selection
            .contains(std::path::Path::new("file.txt")));
        assert_eq!(manager.active().scroll_offset_y, 120.0);
        assert!(manager.tabs[first].items.is_empty());
        assert!(manager.tabs[first].multi_selection.is_empty());
        manager.switch_to(first);
        assert_eq!(manager.active().path, "D:\\");
    }

    #[test]
    fn background_creation_enforces_limit_without_changing_focus() {
        let mut manager = TabManager::new();
        for _ in 1..MAX_TABS {
            assert!(manager.new_background_tab_at("C:\\").is_some());
        }
        assert!(manager.new_background_tab_at("D:\\").is_none());
        assert_eq!(manager.tabs.len(), MAX_TABS);
        assert_eq!(manager.active_tab, 0);
    }
}
