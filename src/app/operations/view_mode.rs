use crate::app::ImageViewerApp;
use crate::domain::file_entry::ViewMode;

impl ImageViewerApp {
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
