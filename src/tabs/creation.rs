use super::{TabManager, TabState, MAX_TABS};

impl TabManager {
    pub(crate) fn can_replace_unpinned_tabs_with(&self, count: usize) -> bool {
        let pinned_count = self.tabs.iter().filter(|tab| tab.pinned).count();
        count > 0
            && count <= MAX_TABS.saturating_sub(pinned_count)
            && self.next_id.checked_add(count).is_some()
    }

    /// Replace every unpinned workspace tab with the given paths. Capacity is
    /// checked before any tab state is changed.
    pub(crate) fn replace_unpinned_tabs_with(&mut self, paths: &[String]) -> bool {
        if !self.can_replace_unpinned_tabs_with(paths.len()) {
            return false;
        }

        let (show_left_sidebar, show_preview_panel, collapse_tags, group_mode, group_descending) = {
            let current = self.active();
            (
                current.show_left_sidebar,
                current.show_preview_panel,
                current.collapse_tags,
                current.group_mode,
                current.group_descending,
            )
        };
        let first_id = self.next_id;
        let mut new_tabs = Vec::with_capacity(paths.len());
        for (offset, path) in paths.iter().enumerate() {
            let mut tab = TabState::new_at_path(first_id + offset, path);
            tab.show_left_sidebar = show_left_sidebar;
            tab.show_preview_panel = show_preview_panel;
            tab.collapse_tags = collapse_tags;
            tab.group_mode = group_mode;
            tab.group_descending = group_descending;
            new_tabs.push(tab);
        }

        self.tabs.retain(|tab| tab.pinned);
        let pinned_count = self.tabs.len();
        self.tabs.extend(new_tabs);
        self.next_id = first_id + paths.len();
        self.active_tab = pinned_count;
        true
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
    fn replacing_tabs_keeps_pinned_tabs_and_activates_the_group_first_path() {
        let mut manager = TabManager::new_at_path(r"C:\Original");
        manager.tabs[0].pinned = true;
        manager.new_tab_at(r"D:\Old");
        manager.new_tab_at(r"E:\Old");

        assert!(
            manager.replace_unpinned_tabs_with(&[r"F:\First".to_owned(), r"G:\Second".to_owned(),])
        );
        assert_eq!(manager.tabs.len(), 3);
        assert!(manager.tabs[0].pinned);
        assert_eq!(manager.tabs[0].path, r"C:\Original");
        assert_eq!(manager.tabs[1].path, r"F:\First");
        assert_eq!(manager.tabs[2].path, r"G:\Second");
        assert_eq!(manager.active_tab, 1);
    }

    #[test]
    fn replacing_tabs_over_capacity_leaves_the_tab_set_unchanged() {
        let mut manager = TabManager::new_at_path(r"C:\Pinned0");
        manager.tabs[0].pinned = true;
        for index in 1..MAX_TABS {
            assert!(manager
                .new_background_tab_at(&format!(r"C:\Pinned{index}"))
                .is_some());
            manager.tabs[index].pinned = true;
        }
        let original_paths: Vec<_> = manager.tabs.iter().map(|tab| tab.path.clone()).collect();
        let original_active = manager.active_tab;
        let original_next_id = manager.next_id;

        assert!(!manager.replace_unpinned_tabs_with(&[r"D:\New".to_owned()]));
        assert_eq!(
            manager
                .tabs
                .iter()
                .map(|tab| tab.path.clone())
                .collect::<Vec<_>>(),
            original_paths
        );
        assert_eq!(manager.active_tab, original_active);
        assert_eq!(manager.next_id, original_next_id);
    }
}
