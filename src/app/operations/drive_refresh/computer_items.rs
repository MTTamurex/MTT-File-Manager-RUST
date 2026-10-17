use crate::domain::file_entry::{DriveInfo, FileEntry};
use crate::infrastructure::windows::normalize_drive_root_path;
use std::path::Path;

pub(super) fn apply_drive_info(items: &mut [FileEntry], results: &[(String, DriveInfo)]) {
    for item in items {
        // Saved shortcuts live on a local disk, but are not entries for that disk.
        if item.drive_info.is_none() {
            continue;
        }
        let Some(root) = normalize_drive_root_path(&item.path) else {
            continue;
        };
        if let Some((_, info)) = results
            .iter()
            .find(|(path, _)| normalize_drive_root_path(Path::new(path)).as_ref() == Some(&root))
        {
            item.drive_info = Some(info.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::grouping::{build_computer_projection, GroupKey};
    use crate::domain::network_location::{NetworkLocation, NetworkLocationKind};
    use crate::infrastructure::windows::DriveType;

    fn info(kind: DriveType, total_space: u64) -> DriveInfo {
        DriveInfo {
            drive_type: kind,
            file_system: "NTFS".into(),
            total_space,
            free_space: total_space / 2,
            model: None,
            serial_number: None,
            firmware_revision: None,
            bus_type: None,
            health: None,
        }
    }

    fn entry(path: &str) -> FileEntry {
        NetworkLocation {
            kind: NetworkLocationKind::FolderShortcut,
            shortcut_path: path.into(),
            path: r"\\server\share".into(),
            label: "Network Folder".into(),
        }
        .to_file_entry()
    }

    #[test]
    fn capacity_refresh_keeps_saved_locations_separate_from_local_and_mapped_drives() {
        let mut local = entry(r"C:\");
        local.drive_info = Some(info(DriveType::Fixed, 0));
        let mut mapped = entry(r"Z:\");
        mapped.drive_info = Some(info(DriveType::Remote, 0));
        let location = entry(r"C:\Network Shortcuts\Network Folder");
        let mut items = vec![local, location, mapped];
        apply_drive_info(
            &mut items,
            &[
                ("c:".into(), info(DriveType::Fixed, 1000)),
                ("z:".into(), info(DriveType::Remote, 2000)),
            ],
        );
        assert_eq!(items[0].drive_info.as_ref().unwrap().total_space, 1000);
        assert!(items[1].drive_info.is_none());
        assert_eq!(items[2].drive_info.as_ref().unwrap().total_space, 2000);
        let projection = build_computer_projection(&items);
        assert_eq!(projection.sections[0].key, GroupKey::LocalDrives);
        assert_eq!(projection.sections[0].item_indices.as_ref(), &[0]);
        assert_eq!(projection.sections[1].key, GroupKey::NetworkDrives);
        assert_eq!(projection.sections[1].item_indices.as_ref(), &[2]);
        assert_eq!(projection.sections[2].key, GroupKey::NetworkLocations);
        assert_eq!(projection.sections[2].item_indices.as_ref(), &[1]);
    }

    #[test]
    fn child_paths_never_match_a_volume_refresh() {
        let mut item = entry(r"C:\folder");
        item.drive_info = Some(info(DriveType::Fixed, 0));
        apply_drive_info(
            std::slice::from_mut(&mut item),
            &[(r"C:\".into(), info(DriveType::Fixed, 1000))],
        );
        assert_eq!(item.drive_info.as_ref().unwrap().total_space, 0);
        item.path = r"C:\".into();
        apply_drive_info(
            std::slice::from_mut(&mut item),
            &[(r"C:\folder".into(), info(DriveType::Fixed, 1000))],
        );
        assert_eq!(item.drive_info.as_ref().unwrap().total_space, 0);
    }
}
