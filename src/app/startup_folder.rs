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

pub(crate) fn is_valid_saved_tab_path(path: &str) -> bool {
    if path == crate::domain::special_paths::RECYCLE_BIN_VIEW_ID {
        return true;
    }

    let path_buf = PathBuf::from(path);
    if let Some((archive_file, _)) = crate::domain::file_entry::split_archive_path(&path_buf) {
        if onedrive::fast_path_exists(&archive_file) && !onedrive::fast_is_dir(&archive_file) {
            return true;
        }
    }
    if path_buf
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(crate::domain::file_entry::is_archive_extension)
        && onedrive::fast_path_exists(&path_buf)
        && !onedrive::fast_is_dir(&path_buf)
    {
        return true;
    }

    is_valid_startup_folder_path(path)
}

pub(crate) fn validate_requested_folder(folder: PathBuf) -> Result<String, ()> {
    let path = folder.to_string_lossy().into_owned();
    is_valid_startup_folder_path(&path)
        .then_some(path)
        .ok_or(())
}

#[cfg(test)]
mod tests {
    use super::is_valid_saved_tab_path;

    #[test]
    fn saved_tabs_accept_existing_archive_roots_and_entries() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("sample.zip");
        std::fs::write(&archive, b"archive").unwrap();
        let archive_path = archive.to_string_lossy().into_owned();
        let archive_entry = format!(r"{archive_path}\folder");

        assert!(is_valid_saved_tab_path(&archive_path));
        assert!(is_valid_saved_tab_path(&archive_entry));
    }

    #[test]
    fn saved_tabs_reject_missing_archive_files() {
        let temp = tempfile::tempdir().unwrap();
        let archive_entry = temp.path().join("missing.zip").join("folder");
        let directory_named_like_archive = temp.path().join("folder.zip");
        std::fs::create_dir(&directory_named_like_archive).unwrap();
        let directory_inside_archive_named_folder = directory_named_like_archive.join("inside");
        std::fs::create_dir(&directory_inside_archive_named_folder).unwrap();

        assert!(!is_valid_saved_tab_path(&archive_entry.to_string_lossy()));
        assert!(is_valid_saved_tab_path(
            &directory_named_like_archive.to_string_lossy()
        ));
        assert!(is_valid_saved_tab_path(
            &directory_inside_archive_named_folder.to_string_lossy()
        ));
    }
}
