//! Trait implementations for App
//!
//! This module implements various UI traits for ImageViewerApp to avoid cluttering other files.

use crate::app::state::ImageViewerApp;
use crate::ui::components::item_slot::ItemSlotOperations;
use crate::ui::context_menu::ContextMenuOperations;

impl ItemSlotOperations for ImageViewerApp {
    fn request_thumbnail_load(
        &mut self,
        path: std::path::PathBuf,
        size: u32,
        directory_index: Option<usize>,
    ) {
        if let Some(index) = directory_index {
            ImageViewerApp::request_thumbnail_load_with_index(self, path, size, index);
        } else {
            ImageViewerApp::request_thumbnail_load(self, path, size);
        }
    }

    fn request_folder_scan(&mut self, path: std::path::PathBuf) {
        // Call inherent method directly
        ImageViewerApp::request_folder_scan(self, path);
    }

    fn request_folder_preview_load(&mut self, path: std::path::PathBuf) {
        ImageViewerApp::request_folder_preview_load(self, path);
    }

    fn request_icon_load(&mut self, path: std::path::PathBuf) {
        ImageViewerApp::request_icon_load(self, path);
    }

    fn rename_item(&mut self, idx: usize) {
        self.rename_with_shell(idx);
    }
}

impl ContextMenuOperations for ImageViewerApp {
    fn create_new_folder(&mut self) {
        self.create_new_folder();
    }

    fn command_copy(&mut self, idx: Option<usize>) {
        self.command_copy(idx);
    }

    fn command_cut(&mut self, idx: Option<usize>) {
        self.command_cut(idx);
    }

    fn command_paste(&mut self, idx: Option<usize>) {
        self.command_paste(idx);
    }

    fn rename_item(&mut self, idx: usize) {
        if let Some(item) = self.items.get(idx) {
            self.renaming_state = Some((idx, item.name.clone()));
            self.focus_rename = true;
        }
    }

    fn delete_with_shell(&mut self, idx: Option<usize>) {
        self.delete_with_shell_for_idx(idx);
    }
}
