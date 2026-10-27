use std::path::Path;

/// Consistency probes inspect real directories, never virtual views. Use the
/// same policy for both panels and for delayed results received after a switch.
pub(super) fn is_filesystem_probe_target(path: &str) -> bool {
    !path.is_empty()
        && !crate::domain::special_paths::is_virtual_path(path)
        && crate::domain::network_path::unc_server_name(path).is_none()
        && !crate::domain::file_entry::is_path_inside_archive(Path::new(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::special_paths::{tag_view_path, COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};

    #[test]
    fn neither_panel_can_probe_tags_or_other_virtual_views() {
        for path in [
            COMPUTER_VIEW_ID,
            RECYCLE_BIN_VIEW_ID,
            "",
            r"\\server",
            r"C:\Documents\archive.zip\Folder",
        ] {
            assert!(!is_filesystem_probe_target(path), "{path}");
        }
        for id in 1..=5 {
            assert!(!is_filesystem_probe_target(&tag_view_path(id)));
        }
    }

    #[test]
    fn real_folders_remain_eligible_without_checking_their_existence() {
        for path in [r"C:\", r"C:\Documents", r"\\server\share\Folder"] {
            assert!(is_filesystem_probe_target(path), "{path}");
        }
    }

    #[test]
    fn a_delayed_tag_result_stays_ineligible_after_that_tag_becomes_active() {
        let left = (1..=4).map(tag_view_path).collect::<Vec<_>>();
        let right = tag_view_path(5);
        for selected in &left {
            // A stale response keeps its original path even if focus moved from
            // the right panel to this pinned left tab before it was received.
            let delayed_result_path = selected.clone();
            assert_ne!(delayed_result_path, right);
            let current_path = selected;
            assert_eq!(&delayed_result_path, current_path);
            assert!(!is_filesystem_probe_target(&delayed_result_path));
        }
    }
}
