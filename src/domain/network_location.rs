use crate::domain::file_entry::{FileEntry, SyncStatus};
use crate::domain::file_tag::normalize_tag_path_key;
use std::path::{Path, PathBuf};

/// A filesystem destination saved in Windows' Network Locations folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkLocation {
    /// Local saved shortcut (folder or .lnk); file operations use this identity.
    pub shortcut_path: String,
    /// Resolved destination; navigation and child creation use this path.
    pub path: String,
    pub label: String,
    pub kind: NetworkLocationKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkLocationKind {
    FolderShortcut,
    LinkFile,
}

pub fn network_location_path_key(path: &Path) -> String {
    normalize_tag_path_key(path)
        .trim_end_matches('\\')
        .to_string()
}

impl NetworkLocation {
    pub fn rename_filename(&self, requested: &str) -> String {
        if self.kind == NetworkLocationKind::LinkFile
            && !requested.is_empty()
            && !Path::new(requested)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
        {
            format!("{requested}.lnk")
        } else {
            requested.to_owned()
        }
    }

    pub fn apply_shortcut_rename(&mut self, new_path: &Path) {
        self.shortcut_path = new_path.to_string_lossy().into_owned();
        let label = match self.kind {
            NetworkLocationKind::FolderShortcut => new_path.file_name(),
            NetworkLocationKind::LinkFile => new_path.file_stem(),
        };
        if let Some(label) = label {
            self.label = label.to_string_lossy().into_owned();
        }
    }

    pub fn matches_shortcut_path(&self, path: &Path) -> bool {
        network_location_path_key(Path::new(&self.shortcut_path)) == network_location_path_key(path)
    }

    /// Build a navigable entry without contacting the server for metadata.
    pub fn to_file_entry(&self) -> FileEntry {
        FileEntry {
            path: PathBuf::from(&self.shortcut_path),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_file_renames_preserve_the_link_extension_and_the_remote_destination() {
        let mut location = NetworkLocation {
            shortcut_path: r"C:\Network Shortcuts\Saved Link.lnk".into(),
            path: r"\\server\share".into(),
            label: "Saved Link".into(),
            kind: NetworkLocationKind::LinkFile,
        };
        assert_eq!(location.rename_filename("Renamed Link"), "Renamed Link.lnk");
        assert_eq!(
            location.rename_filename("Renamed Link.LNK"),
            "Renamed Link.LNK"
        );
        assert_eq!(location.rename_filename(""), "");
        assert_eq!(location.rename_filename(".lnk"), ".lnk.lnk");
        location.apply_shortcut_rename(Path::new(r"C:\Network Shortcuts\Renamed Link.lnk"));
        assert_eq!(location.label, "Renamed Link");
        assert_eq!(location.path, r"\\server\share");
        assert_eq!(location.rename_filename("Another Link"), "Another Link.lnk");
    }

    #[test]
    fn folder_shortcuts_named_like_link_files_remain_folders() {
        let location = NetworkLocation {
            shortcut_path: r"C:\Network Shortcuts\Folder.lnk".into(),
            path: r"\\server\share".into(),
            label: "Folder.lnk".into(),
            kind: NetworkLocationKind::FolderShortcut,
        };
        assert_eq!(location.rename_filename("Renamed Folder"), "Renamed Folder");
    }
}
