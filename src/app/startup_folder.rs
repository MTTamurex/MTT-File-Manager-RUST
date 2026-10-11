use std::path::PathBuf;

use crate::domain::special_paths::{is_virtual_path, COMPUTER_VIEW_ID};
use crate::infrastructure::onedrive;

pub(crate) fn is_valid_startup_folder_path(path: &str) -> bool {
    if path == COMPUTER_VIEW_ID || crate::domain::special_paths::is_tag_view_path(path) {
        return true;
    }

    if path.is_empty() || path.starts_with("shell:") || is_virtual_path(path) {
        return false;
    }

    let path_buf = PathBuf::from(path);
    onedrive::fast_path_exists(&path_buf) && onedrive::fast_is_dir(&path_buf)
}

pub(crate) fn validate_requested_folder(folder: PathBuf) -> Result<String, ()> {
    let path = folder.to_string_lossy().into_owned();
    is_valid_startup_folder_path(&path)
        .then_some(path)
        .ok_or(())
}
