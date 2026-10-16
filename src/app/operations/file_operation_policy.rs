//! Destination policy shared by commands, menus and drag-and-drop.

use crate::app::state::ImageViewerApp;
use crate::domain::network_location::NetworkLocation;
use crate::domain::network_path::{is_network_location_root, unc_server_name};
use crate::domain::special_paths::is_virtual_path;
use std::path::Path;

/// This checks the kind of destination, not its filesystem permissions.
/// The filesystem operation remains responsible for enforcing write access.
pub(crate) fn path_accepts_file_children(path: &Path) -> bool {
    let text = path.to_string_lossy();
    !text.is_empty()
        && !is_virtual_path(&text)
        && unc_server_name(&text).is_none()
        && !ImageViewerApp::path_is_archive_namespace(path)
}

impl ImageViewerApp {
    pub(crate) fn path_is_network_operation_root(&self, path: &Path) -> bool {
        network_operation_root(
            path,
            self.navigation_state.is_computer_view && !self.global_search.active,
            &self.drive_state.network_locations,
        )
    }

    pub(crate) fn can_create_folder_in_current_location(&self) -> bool {
        path_accepts_file_children(Path::new(&self.navigation_state.current_path))
    }
}

fn network_operation_root(
    path: &Path,
    is_computer_view: bool,
    locations: &[NetworkLocation],
) -> bool {
    is_network_location_root(path)
        || (is_computer_view && locations.iter().any(|location| location.matches_path(path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_network_location_cards_do_not_mutate_their_target_folders() {
        let locations = vec![NetworkLocation {
            path: r"\\server\share\folder".into(),
            label: "Network Folder".into(),
        }];
        let target = Path::new(r"\\SERVER\share\folder\");
        assert!(network_operation_root(target, true, &locations));
        assert!(!network_operation_root(target, false, &locations));
        assert!(!network_operation_root(
            Path::new(r"\\server\share\folder\child"),
            true,
            &locations
        ));
        assert!(network_operation_root(
            Path::new(r"\\server\share"),
            false,
            &locations
        ));
    }

    #[test]
    fn network_server_is_not_a_file_destination_but_a_share_is() {
        for path in [r"\\server", r"\\server\"] {
            assert!(!path_accepts_file_children(Path::new(path)), "{path}");
        }
        for path in [
            r"\\server\share",
            r"\\server\share\",
            r"\\server\share\folder",
            r"\\?\UNC\server\share",
            r"C:\Folder",
        ] {
            assert!(path_accepts_file_children(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn file_destinations_still_exclude_virtual_views_and_archives() {
        for path in ["", "::computer::", "::recycle_bin::", "::tag::1"] {
            assert!(!path_accepts_file_children(Path::new(path)), "{path}");
        }
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("archive.zip");
        std::fs::write(&archive, b"archive").unwrap();
        assert!(!path_accepts_file_children(&archive));
        assert!(!path_accepts_file_children(&archive.join("folder")));
        let folder = temp.path().join("folder.zip");
        std::fs::create_dir(&folder).unwrap();
        assert!(path_accepts_file_children(&folder));
    }
}
