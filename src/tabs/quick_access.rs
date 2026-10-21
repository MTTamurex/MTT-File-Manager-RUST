//! Quick Access policy shared by both tab strips and their action handlers.
use crate::domain::pinned_folder::PinnedFolder;
use crate::domain::special_paths::is_virtual_path;
use crate::tabs::TabAction;

pub(crate) fn tab_quick_access_action(path: &str, pinned: &[PinnedFolder]) -> Option<TabAction> {
    // Tab destinations are directories; do not probe disk/network paths on the UI thread.
    if is_virtual_path(path) || !std::path::Path::new(path).is_absolute() {
        return None;
    }
    Some(if pinned_folder_for_path(path, pinned).is_some() {
        TabAction::UnpinQuickAccess
    } else {
        TabAction::PinQuickAccess
    })
}

pub(crate) fn pinned_folder_for_path<'a>(
    path: &str,
    pinned: &'a [PinnedFolder],
) -> Option<&'a PinnedFolder> {
    use crate::domain::file_tag::normalize_tag_path_key;
    use std::path::Path;
    let key = normalize_tag_path_key(Path::new(path));
    pinned
        .iter()
        .find(|folder| normalize_tag_path_key(Path::new(&folder.path)) == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::special_paths::{tag_view_path, COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};

    #[test]
    fn folders_and_drives_offer_pin_or_unpin_without_checking_disk() {
        for path in [r"C:\", r"C:\Example\Folder", r"\\server\share\folder"] {
            assert_eq!(
                tab_quick_access_action(path, &[]),
                Some(TabAction::PinQuickAccess)
            );
            let pinned = vec![PinnedFolder {
                path: path.into(),
                display_name: "Example".into(),
                position: 0,
            }];
            assert_eq!(
                tab_quick_access_action(path, &pinned),
                Some(TabAction::UnpinQuickAccess)
            );
        }
    }

    #[test]
    fn virtual_and_invalid_destinations_have_no_quick_access_command() {
        for path in [
            COMPUTER_VIEW_ID,
            RECYCLE_BIN_VIEW_ID,
            &tag_view_path(1),
            "",
            "relative",
        ] {
            assert_eq!(tab_quick_access_action(path, &[]), None);
        }
    }

    #[test]
    fn equivalent_windows_paths_reuse_the_saved_quick_access_entry() {
        let pinned = vec![PinnedFolder {
            path: r"C:\Example\Folder".into(),
            display_name: "Example".into(),
            position: 0,
        }];
        for path in [
            r"c:\example\folder\",
            "C:/Example/Folder",
            r"\\?\C:\Example\Folder",
        ] {
            assert_eq!(
                tab_quick_access_action(path, &pinned),
                Some(TabAction::UnpinQuickAccess)
            );
            assert_eq!(
                pinned_folder_for_path(path, &pinned).unwrap().path,
                pinned[0].path
            );
        }
    }
}
