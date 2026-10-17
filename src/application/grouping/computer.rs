use super::{GroupKey, GroupProjection, GroupSection};
use crate::domain::file_entry::FileEntry;
use crate::infrastructure::windows::DriveType;

pub fn build_computer_projection(items: &[FileEntry]) -> GroupProjection {
    let mut local = Vec::new();
    let mut network = Vec::new();
    let mut locations = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match item.drive_info.as_ref().map(|drive| drive.drive_type) {
            Some(DriveType::Remote) => network.push(index),
            None if item.is_dir => locations.push(index),
            _ => local.push(index),
        }
    }
    let mut sections = Vec::with_capacity(3);
    for (key, indices) in [
        (GroupKey::LocalDrives, local),
        (GroupKey::NetworkDrives, network),
        (GroupKey::NetworkLocations, locations),
    ] {
        if !indices.is_empty() {
            sections.push(GroupSection {
                key,
                item_indices: indices.into(),
            });
        }
    }
    GroupProjection { sections }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::file_entry::DriveInfo;
    use crate::domain::network_location::NetworkLocation;

    fn drive(path: &str, kind: DriveType) -> FileEntry {
        let mut item = NetworkLocation {
            kind: crate::domain::network_location::NetworkLocationKind::FolderShortcut,
            shortcut_path: path.into(),
            path: path.into(),
            label: "Drive".into(),
        }
        .to_file_entry();
        item.drive_info = Some(DriveInfo {
            drive_type: kind,
            file_system: String::new(),
            total_space: 0,
            free_space: 0,
            model: None,
            serial_number: None,
            firmware_revision: None,
            bus_type: None,
            health: None,
        });
        item
    }

    #[test]
    fn network_locations_have_their_own_section_and_follow_filtered_item_indices() {
        let location = NetworkLocation {
            kind: crate::domain::network_location::NetworkLocationKind::FolderShortcut,
            shortcut_path: r"C:\Network Shortcuts\Network Folder".into(),
            path: r"\\server\share\folder".into(),
            label: "Network Folder".into(),
        }
        .to_file_entry();
        let items = vec![
            location.clone(),
            drive(r"Z:\", DriveType::Remote),
            drive(r"C:\", DriveType::Fixed),
        ];
        let projection = build_computer_projection(&items);
        assert_eq!(
            projection
                .sections
                .iter()
                .map(|section| section.key.clone())
                .collect::<Vec<_>>(),
            vec![
                GroupKey::LocalDrives,
                GroupKey::NetworkDrives,
                GroupKey::NetworkLocations
            ]
        );
        assert_eq!(projection.sections[2].item_indices.as_ref(), &[0]);
        let filtered = build_computer_projection(&[location]);
        assert_eq!(filtered.sections.len(), 1);
        assert_eq!(filtered.sections[0].key, GroupKey::NetworkLocations);
        assert_eq!(filtered.sections[0].item_indices.as_ref(), &[0]);
    }
}
