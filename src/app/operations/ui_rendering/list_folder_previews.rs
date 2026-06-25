use std::path::{Path, PathBuf};

use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::FileEntry;
use crate::ui::cache::FxHashSet;

impl ImageViewerApp {
    pub(crate) fn idle_folder_preview_keep_count(&self) -> usize {
        usize::from(self.detail_panel_folder_preview_path().is_some())
    }

    pub(crate) fn detail_panel_folder_preview_paths_for_trim(&self) -> Option<FxHashSet<PathBuf>> {
        let path = self.detail_panel_folder_preview_path()?;
        let mut paths = FxHashSet::default();
        paths.insert(path);
        Some(paths)
    }

    pub(crate) fn warm_detail_panel_folder_preview(&mut self) {
        let Some(path) = self.detail_panel_folder_preview_path() else {
            return;
        };

        if self.cache_manager.has_folder_preview(&path)
            || self.cache_manager.is_folder_preview_loading(&path)
        {
            return;
        }

        if self.cache_manager.folder_preview_cache.cap().get() < 1 {
            self.cache_manager.retune_folder_preview_cache_capacity(1);
        }

        self.request_folder_preview_load(path);
    }

    pub(crate) fn detail_panel_folder_preview_path(&self) -> Option<PathBuf> {
        if !self.show_preview_panel
            || self.multi_selection.len() > 1
            || self.navigation_state.is_computer_view
            || self.navigation_state.is_recycle_bin_view
            || crate::infrastructure::windows::is_windows_system_path(
                &self.navigation_state.current_path,
            )
        {
            return None;
        }

        if crate::domain::special_paths::is_virtual_path(&self.navigation_state.current_path) {
            return None;
        }

        if let Some(selected) = self.selected_file.as_ref() {
            return should_use_composed_folder_preview(selected).then(|| selected.path.clone());
        }

        let path = PathBuf::from(&self.navigation_state.current_path);
        should_use_composed_folder_preview_path(&path, true).then_some(path)
    }
}

fn should_use_composed_folder_preview(item: &FileEntry) -> bool {
    if !item.is_dir || item.is_archive() {
        return false;
    }

    should_use_composed_folder_preview_path(&item.path, item.is_dir)
}

fn should_use_composed_folder_preview_path(path: &Path, is_dir: bool) -> bool {
    let path_text = path.to_string_lossy();
    !crate::domain::special_paths::is_virtual_path(path_text.as_ref())
        && !crate::infrastructure::windows::is_windows_system_path(path_text.as_ref())
        && !crate::infrastructure::windows::shell_folder::is_shell_navigation_path(path, is_dir)
        && !crate::infrastructure::onedrive::is_special_icon_folder(path)
}
