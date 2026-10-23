//! Refresh policy: missing navigation dates, debounced filesystem events, and F5.
use crate::app::folder_timestamp_state::{ReadLane, RefreshReason};
use crate::app::ImageViewerApp;
use crate::domain::file_tag::normalize_tag_path_key;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn query_lane(
    path: &Path,
    remote_drive: impl FnOnce(&Path) -> bool,
    virtual_drive: impl FnOnce(&Path) -> bool,
) -> Option<ReadLane> {
    let text = path.to_string_lossy();
    // The generic drive filter treats every leading double backslash as UNC.
    // Strip only the extended *drive* prefix for that check; never reinterpret
    // UNC, device or volume namespaces as local drive paths.
    let drive_path = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let bytes = drive_path.as_bytes();
    if bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || !matches!(bytes[2], b'\\' | b'/')
    {
        return None;
    }
    let drive_path = Path::new(drive_path);
    if virtual_drive(drive_path) {
        Some(ReadLane::Virtual)
    } else if !remote_drive(drive_path) {
        Some(ReadLane::Local)
    } else {
        None
    }
}

fn changed_targets(paths: &[PathBuf], known: &HashMap<String, PathBuf>) -> Vec<PathBuf> {
    let mut targets = HashMap::new();
    for path in paths {
        // The event may describe the directory itself or an immediate child.
        // Do not stat paths (or recursively refresh unrelated ancestors) here.
        for candidate in std::iter::once(path.as_path()).chain(path.parent()) {
            let key = normalize_tag_path_key(candidate);
            if let Some(owner) = known.get(&key) {
                targets.insert(key, owner.clone());
            }
        }
    }
    targets.into_values().collect()
}

impl ImageViewerApp {
    pub(in crate::app::operations) fn queue_folder_timestamp_refresh(
        &mut self,
        path: &Path,
        reason: RefreshReason,
    ) {
        let Some(lane) = query_lane(
            path,
            |candidate| {
                use crate::infrastructure::windows::{detect_drive_type, DriveType};
                // query_lane already validated an absolute drive-letter path.
                // Check only its drive type here; do not probe virtual markers
                // a second time through is_network_or_virtual.
                let text = candidate.to_string_lossy();
                matches!(
                    detect_drive_type(&text[..2]),
                    DriveType::Remote | DriveType::Unknown
                )
            },
            crate::infrastructure::io_priority::is_virtual_drive_path,
        ) else {
            return;
        };
        if self
            .folder_timestamps
            .request(path, reason, Instant::now(), lane)
        {
            self.ui_ctx.request_repaint();
        }
    }

    pub(in crate::app::operations) fn folder_timestamp_targets(
        &mut self,
    ) -> HashMap<String, PathBuf> {
        let mut targets = HashMap::new();
        for (path, _) in self
            .folder_modified_hints
            .iter()
            .chain(self.folder_created_hints.iter())
        {
            targets.insert(normalize_tag_path_key(path), path.clone());
        }
        let path = PathBuf::from(&self.navigation_state.current_path);
        targets.insert(normalize_tag_path_key(&path), path);
        self.visit_stored_folder_timestamps(|path, _, _| {
            let path = PathBuf::from(path);
            targets.insert(normalize_tag_path_key(&path), path);
        });
        targets
    }

    pub(in crate::app::operations) fn queue_folder_timestamp_changes(
        &mut self,
        paths: &[PathBuf],
        targets: &HashMap<String, PathBuf>,
    ) {
        for path in changed_targets(paths, targets) {
            self.queue_folder_timestamp_refresh(&path, RefreshReason::Watcher);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_paths_refresh_only_known_directories_and_their_immediate_parents() {
        let folders = [r"C:\Folder", r"C:\Folder\Child", r"D:\Other"];
        let known = folders
            .into_iter()
            .map(|p| (normalize_tag_path_key(Path::new(p)), PathBuf::from(p)))
            .collect();
        let events = [
            PathBuf::from(r"c:\folder\child\file.txt"),
            PathBuf::from(r"C:\Folder\Child"),
        ];
        let targets = changed_targets(&events, &known);
        assert_eq!(targets.len(), 2);
        assert!(targets.contains(&PathBuf::from(folders[0])));
        assert!(targets.contains(&PathBuf::from(folders[1])));
        assert!(!targets.contains(&PathBuf::from(folders[2])));
    }

    #[test]
    fn rename_refreshes_both_parents_and_the_new_directory_owner() {
        let folders = [r"C:\Source", r"D:\Destination", r"D:\Destination\Folder"];
        let known = folders
            .into_iter()
            .map(|p| (normalize_tag_path_key(Path::new(p)), PathBuf::from(p)))
            .collect();
        let events = [
            PathBuf::from(r"C:\Source\Folder"),
            PathBuf::from(r"D:\Destination\Folder"),
        ];
        let targets = changed_targets(&events, &known);
        assert_eq!(targets.len(), 3);
    }

    #[test]
    fn namespace_relative_and_network_paths_are_not_queried() {
        for path in [
            "",
            "::computer::",
            "::recycle_bin::",
            "::tag::1",
            "shell:Downloads",
            "relative",
            r"\\server\share\Folder",
            r"\\?\UNC\server\share\Folder",
            r"\\.\C:\Folder",
            r"\\?\Volume{00000000-0000-0000-0000-000000000000}\Folder",
        ] {
            assert!(query_lane(
                Path::new(path),
                |_| panic!("must reject before drive lookup"),
                |_| panic!("must reject before virtual drive lookup"),
            )
            .is_none());
        }
        for path in [r"\\server\share\Folder", r"Z:\Folder"] {
            assert!(query_lane(Path::new(path), |_| true, |_| false).is_none());
        }
        assert_eq!(
            query_lane(Path::new(r"C:\Folder"), |_| false, |_| false),
            Some(ReadLane::Local)
        );
    }

    #[test]
    fn extended_local_alias_is_checked_as_a_drive_path_not_as_a_network_share() {
        assert_eq!(
            query_lane(
                Path::new(r"\\?\C:\Folder"),
                |candidate| { candidate.to_string_lossy().starts_with(r"\\") },
                |_| false
            ),
            Some(ReadLane::Local)
        );
    }

    #[test]
    fn real_directories_named_like_archives_are_not_filtered_by_their_names() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("folder.zip").join("Child");
        std::fs::create_dir_all(&child).unwrap();
        assert_eq!(
            query_lane(&child, |_| false, |_| false),
            Some(ReadLane::Local)
        );
    }

    #[test]
    fn cryptomator_drive_uses_isolated_lane_instead_of_being_rejected() {
        for path in [r"Z:\Folder", r"\\?\Z:\Folder"] {
            assert_eq!(
                query_lane(
                    Path::new(path),
                    |_| true,
                    |candidate| {
                        assert_eq!(candidate, Path::new(r"Z:\Folder"));
                        true
                    }
                ),
                Some(ReadLane::Virtual)
            );
        }
    }
}
