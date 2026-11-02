use crate::app::init_workers::consistency_probe_worker::ConsistencyProbeResult;
use std::path::Path;
use std::sync::mpsc;

/// Consistency probes inspect real directories, never virtual views. Use the
/// same policy for both panels and for delayed results received after a switch.
pub(super) fn is_filesystem_probe_target(path: &str) -> bool {
    !path.is_empty()
        && !crate::domain::special_paths::is_virtual_path(path)
        && crate::domain::network_path::unc_server_name(path).is_none()
        && !crate::domain::file_entry::is_path_inside_archive(Path::new(path))
}

/// Discard stale virtual-view responses before they can reach filesystem recovery.
pub(super) fn next_filesystem_probe_result(
    receiver: &mpsc::Receiver<ConsistencyProbeResult>,
) -> Option<ConsistencyProbeResult> {
    receiver
        .try_iter()
        .find(|result| is_filesystem_probe_target(&result.path.to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::init_workers::consistency_probe_worker::ConsistencyProbeMode;
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
    fn delayed_virtual_results_are_discarded_without_losing_filesystem_results() {
        let (sender, receiver) = mpsc::channel();
        for mode in [
            ConsistencyProbeMode::PathLiveness,
            ConsistencyProbeMode::ListingDrift,
            ConsistencyProbeMode::FolderContents,
        ] {
            for path in (1..=5)
                .map(tag_view_path)
                .chain([COMPUTER_VIEW_ID.to_owned(), RECYCLE_BIN_VIEW_ID.to_owned()])
            {
                // Even an obsolete response claiming disappearance must be ignored.
                sender.send(result(&path, mode, true)).unwrap();
            }
            sender.send(result(r"C:\Documents", mode, true)).unwrap();
            sender.send(result(&tag_view_path(1), mode, false)).unwrap();
            sender.send(result(r"D:\Videos", mode, false)).unwrap();

            let missing = next_filesystem_probe_result(&receiver).unwrap();
            assert_eq!(missing.path, Path::new(r"C:\Documents"));
            assert_eq!(missing.mode, mode);
            assert!(missing.path_vanished);
            let alive = next_filesystem_probe_result(&receiver).unwrap();
            assert_eq!(alive.path, Path::new(r"D:\Videos"));
            assert_eq!(alive.mode, mode);
            assert!(!alive.path_vanished);
            assert!(next_filesystem_probe_result(&receiver).is_none());
        }

        sender
            .send(result(
                &tag_view_path(1),
                ConsistencyProbeMode::PathLiveness,
                true,
            ))
            .unwrap();
        drop(sender);
        assert!(next_filesystem_probe_result(&receiver).is_none());
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
    }

    fn result(
        path: &str,
        mode: ConsistencyProbeMode,
        path_vanished: bool,
    ) -> ConsistencyProbeResult {
        ConsistencyProbeResult {
            path: path.into(),
            mode,
            disk_signature: 0,
            path_vanished,
            changed_folder_covers: Vec::new(),
            changed_folder_contents: Vec::new(),
            content_check_succeeded: false,
        }
    }
}
