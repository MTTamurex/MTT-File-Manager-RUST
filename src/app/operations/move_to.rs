//! Explicit-source folder moves shared by file views, the sidebar and global search.
use super::clipboard_ops::{normalize_path_for_hierarchy, paste_target_is_valid_for_sources};
use super::file_operation_policy::path_accepts_file_children;
use crate::app::state::ImageViewerApp;
use crate::domain::network_path::is_network_location_root;
use crate::infrastructure::security::classify_shell_namespace_path;
use crate::workers::file_operation_worker::FileOperationRequest;
use std::path::{Path, PathBuf};

pub(super) fn supports_sources(paths: &[PathBuf]) -> bool {
    !paths.is_empty()
        && paths.iter().all(|path| {
            path.is_absolute()
                && classify_shell_namespace_path(path).is_none()
                && !crate::infrastructure::windows::is_drive_root_path(path)
                && !is_network_location_root(path)
                && !crate::domain::file_entry::is_path_inside_existing_archive_file(path)
        })
}

fn sources_outside_destination(paths: &[PathBuf], destination: &Path) -> Vec<PathBuf> {
    let destination = normalize_path_for_hierarchy(destination);
    paths
        .iter()
        .filter(|path| {
            !path
                .parent()
                .is_some_and(|parent| normalize_path_for_hierarchy(parent) == destination)
        })
        .cloned()
        .collect()
}

impl ImageViewerApp {
    pub(crate) fn can_move_paths_to_folder(&self, paths: &[PathBuf]) -> bool {
        supports_sources(paths) && self.can_delete_with_shell_paths(paths)
    }

    pub(crate) fn move_paths_to_folder(&mut self, paths: &[PathBuf]) {
        if !self.can_move_paths_to_folder(paths) {
            self.notifications
                .warning(rust_i18n::t!("operations.selection_operation_blocked").to_string());
            return;
        }
        let Some(destination) = rfd::FileDialog::new()
            .set_title(rust_i18n::t!("context_menu.move_to").as_ref())
            .pick_folder()
        else {
            return;
        };
        if !path_accepts_file_children(&destination)
            || !paste_target_is_valid_for_sources(paths, &destination)
        {
            self.notifications
                .warning(rust_i18n::t!("operations.invalid_move_destination").to_string());
            return;
        }
        let sources = sources_outside_destination(paths, &destination);
        if sources.is_empty() {
            return;
        }
        let request = FileOperationRequest::move_batch(sources, destination, self.shell_op_hwnd());
        self.file_operation_state.file_ops_in_progress += 1;
        if self
            .file_operation_state
            .file_op_sender
            .send(request)
            .is_err()
        {
            self.file_operation_state.file_ops_in_progress = self
                .file_operation_state
                .file_ops_in_progress
                .saturating_sub(1);
            self.notifications
                .warning(rust_i18n::t!("operations.error_worker_unavailable").to_string());
        }
    }
}

#[cfg(test)]
#[path = "move_to/tests.rs"]
mod tests;
