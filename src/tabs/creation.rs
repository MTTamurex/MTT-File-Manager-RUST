use super::{TabManager, TabState, MAX_TABS};

impl TabManager {
    pub(crate) fn can_append_tabs(&self, count: usize) -> bool {
        count > 0
            && count <= MAX_TABS.saturating_sub(self.tabs.len())
            && self.next_id.checked_add(count).is_some()
    }

    /// Append fully prepared tabs without changing or removing existing tabs.
    /// Returns the index of the first appended tab when capacity permits.
    pub(crate) fn append_tabs_with_states(&mut self, mut tabs: Vec<TabState>) -> Option<usize> {
        if !self.can_append_tabs(tabs.len()) {
            return None;
        }

        let first_index = self.tabs.len();
        let first_id = self.next_id;
        for (offset, tab) in tabs.iter_mut().enumerate() {
            tab.id = first_id + offset;
            tab.pinned = false;
        }
        self.tabs.extend(tabs);
        self.next_id = first_id + (self.tabs.len() - first_index);
        Some(first_index)
    }

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

    #[test]
    fn appending_prepared_tabs_preserves_existing_tabs_and_adds_after_them() {
        let mut manager = TabManager::new_at_path(r"C:\Users\user\Documents\Project");
        manager.tabs[0].pinned = true;
        manager.new_tab_at(r"D:\Videos\Folder");
        let original_ids = manager.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>();
        let original_active = manager.active_tab;

        let first_added = manager
            .append_tabs_with_states(vec![
                TabState::new_at_path(0, r"E:\Media\FolderA"),
                TabState::new_at_path(0, r"F:\Media\FolderB"),
            ])
            .unwrap();

        assert_eq!(first_added, 2);
        assert_eq!(manager.tabs.len(), 4);
        assert_eq!(manager.tabs[0].id, original_ids[0]);
        assert!(manager.tabs[0].pinned);
        assert_eq!(manager.tabs[1].id, original_ids[1]);
        assert_eq!(manager.tabs[0].path, r"C:\Users\user\Documents\Project");
        assert_eq!(manager.tabs[1].path, r"D:\Videos\Folder");
        assert_eq!(manager.tabs[2].path, r"E:\Media\FolderA");
        assert_eq!(manager.tabs[3].path, r"F:\Media\FolderB");
        assert_eq!(manager.tabs[2].id, 2);
        assert_eq!(manager.tabs[3].id, 3);
        assert_eq!(manager.active_tab, original_active);
        assert!(!manager.tabs[2].pinned);
        assert!(!manager.tabs[3].pinned);
    }

    #[test]
    fn append_over_capacity_leaves_all_existing_tabs_unchanged() {
        let mut manager = TabManager::new_at_path(r"C:\Users\user\Documents\Project");
        while manager.tabs.len() < MAX_TABS - 1 {
            manager.new_tab_at(r"D:\Videos\Folder");
        }
        let original_paths = manager
            .tabs
            .iter()
            .map(|tab| tab.path.clone())
            .collect::<Vec<_>>();
        let original_ids = manager.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>();
        let original_active = manager.active_tab;
        let original_next_id = manager.next_id;

        assert!(manager
            .append_tabs_with_states(vec![
                TabState::new_at_path(0, r"E:\Media\FolderA"),
                TabState::new_at_path(0, r"E:\Media\FolderB"),
            ])
            .is_none());

        assert_eq!(manager.tabs.len(), MAX_TABS - 1);
        assert_eq!(
            manager
                .tabs
                .iter()
                .map(|tab| tab.path.clone())
                .collect::<Vec<_>>(),
            original_paths
        );
        assert_eq!(
            manager.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>(),
            original_ids
        );
        assert_eq!(manager.active_tab, original_active);
        assert_eq!(manager.next_id, original_next_id);
    }

    #[test]
    fn append_rejects_empty_groups_and_exhausted_ids_without_changes() {
        let mut manager = TabManager::new();
        assert_eq!(manager.append_tabs_with_states(Vec::new()), None);
        manager.next_id = usize::MAX;
        assert_eq!(
            manager.append_tabs_with_states(vec![TabState::new_at_path(0, "C:\\")]),
            None
        );
        assert_eq!(manager.tabs.len(), 1);
        assert_eq!(manager.active_tab, 0);
        assert_eq!(manager.next_id, usize::MAX);
    }
}
