use crate::app::ImageViewerApp;
use crate::domain::file_entry::ViewMode;

fn inherited_view_mode(current: ViewMode, normal: ViewMode, source_locked: bool) -> ViewMode {
    if source_locked {
        normal
    } else {
        current
    }
}

impl ImageViewerApp {
    /// New tabs inherit user choices, not a source folder's lock override.
    /// Any destination lock is applied separately when the new tab is restored.
    pub(crate) fn new_tab_view_mode(&self) -> ViewMode {
        inherited_view_mode(
            self.view_mode,
            self.view_mode_normal,
            self.current_folder_locked,
        )
    }

    pub(crate) fn change_view_mode(&mut self, mode: ViewMode) -> bool {
        if self.current_folder_locked || self.view_mode == mode {
            return false;
        }
        self.view_mode = mode;
        self.view_mode_normal = mode;
        self.watch_current_folder();
        let selection_is_outside_current_folder = self
            .selected_file
            .as_ref()
            .is_some_and(|selected| !self.items.iter().any(|item| item.path == selected.path));
        if mode != ViewMode::Miller && selection_is_outside_current_folder {
            self.clear_file_view_selection();
        }
        if matches!(
            mode,
            ViewMode::Grid | ViewMode::ColumnList | ViewMode::Miller
        ) {
            self.folder_size_state.cancel_batch();
        }
        self.save_preferences();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tabs::TabManager;

    const MODES: [ViewMode; 4] = [
        ViewMode::Grid,
        ViewMode::List,
        ViewMode::ColumnList,
        ViewMode::Miller,
    ];

    #[test]
    fn new_tab_ignores_locked_source_mode_and_uses_normal_mode() {
        for current in MODES {
            for normal in MODES {
                assert_eq!(inherited_view_mode(current, normal, true), normal);
            }
        }
    }

    #[test]
    fn new_tab_preserves_unlocked_source_even_if_global_normal_differs() {
        // The two panels may have different unlocked view modes.
        for current in MODES {
            for normal in MODES {
                assert_eq!(inherited_view_mode(current, normal, false), current);
            }
        }
    }

    #[test]
    fn new_tabs_from_locked_grid_do_not_change_source_or_existing_tab_modes() {
        for activate in [false, true] {
            let mut tabs = TabManager::new_at_path("C:\\");
            tabs.active_mut().view_mode = ViewMode::Grid;
            let inherited = inherited_view_mode(tabs.active().view_mode, ViewMode::List, true);
            let first = tabs.new_background_tab_at("D:\\").unwrap();
            tabs.tabs[first].view_mode = inherited;
            let second = tabs.new_background_tab_at("E:\\").unwrap();
            tabs.tabs[second].view_mode = inherited;

            assert_eq!(tabs.active_tab, 0);
            if activate {
                tabs.switch_to(first);
            }
            assert_eq!(tabs.tabs[first].view_mode, ViewMode::List);
            assert_eq!(tabs.tabs[second].view_mode, ViewMode::List);
            assert_eq!(tabs.tabs[0].view_mode, ViewMode::Grid);

            // Switching among existing tabs must not recalculate their modes.
            tabs.switch_to(0);
            assert_eq!(tabs.active().view_mode, ViewMode::Grid);
            tabs.switch_to(second);
            assert_eq!(tabs.active().view_mode, ViewMode::List);
        }
    }
}
