use crate::domain::file_entry::{FileEntry, SyncStatus};
use crate::domain::file_tag::normalize_tag_path_key;
use std::path::{Path, PathBuf};

/// A filesystem network destination saved through Windows' Add Network Location wizard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkLocation {
    pub path: String,
    pub label: String,
}

pub fn network_location_path_key(path: &Path) -> String {
    normalize_tag_path_key(path)
        .trim_end_matches('\\')
        .to_string()
}

impl NetworkLocation {
    pub fn matches_path(&self, path: &Path) -> bool {
        network_location_path_key(Path::new(&self.path)) == network_location_path_key(path)
    }

    /// Build a navigable entry without contacting the server for metadata.
    pub fn to_file_entry(&self) -> FileEntry {
        FileEntry {
            path: PathBuf::from(&self.path),
            name: self.label.clone(),
            is_dir: true,
            size: 0,
            modified: 0,
            created: None,
            folder_cover: None,
            drive_info: None,
            sync_status: SyncStatus::None,
            is_hidden: false,
            recycle_bin: None,
        }
    }
}
