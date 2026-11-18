use super::{folder_group::FolderGroup, pinned_folder::PinnedFolder};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuickAccessItem {
    PinnedFolder(String),
    FolderGroup(i64),
}

pub fn move_quick_access_item(items: &mut Vec<QuickAccessItem>, from: usize, to: usize) -> bool {
    if from >= items.len() || from == to {
        return false;
    }

    let item = items.remove(from);
    items.insert(to.min(items.len()), item);
    true
}

pub fn destination_for_quick_access_gap(
    source: usize,
    insertion_index: usize,
    item_count: usize,
) -> usize {
    let insertion_index = insertion_index.min(item_count);
    insertion_index.saturating_sub(if insertion_index > source { 1 } else { 0 })
}

pub fn ordered_quick_access_items(
    pinned_folders: &[PinnedFolder],
    folder_groups: &[FolderGroup],
) -> Vec<QuickAccessItem> {
    let mut items = Vec::with_capacity(pinned_folders.len() + folder_groups.len());
    items.extend(pinned_folders.iter().map(|folder| {
        (
            folder.position,
            0_u8,
            QuickAccessItem::PinnedFolder(folder.path.clone()),
        )
    }));
    items.extend(
        folder_groups
            .iter()
            .map(|group| (group.position, 1_u8, QuickAccessItem::FolderGroup(group.id))),
    );
    items.sort_by_key(|(position, kind, _)| (*position, *kind));
    items.into_iter().map(|(_, _, item)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_items_across_item_kinds_by_final_index() {
        let mut items = vec![
            QuickAccessItem::PinnedFolder("folder-a".to_owned()),
            QuickAccessItem::FolderGroup(7),
            QuickAccessItem::PinnedFolder("folder-b".to_owned()),
        ];

        assert!(move_quick_access_item(&mut items, 0, 2));
        assert_eq!(
            items,
            [
                QuickAccessItem::FolderGroup(7),
                QuickAccessItem::PinnedFolder("folder-b".to_owned()),
                QuickAccessItem::PinnedFolder("folder-a".to_owned()),
            ]
        );
        assert!(!move_quick_access_item(&mut items, 3, 0));
    }

    #[test]
    fn combines_folders_and_groups_by_global_position() {
        let folders = vec![
            PinnedFolder {
                path: "folder-a".to_owned(),
                display_name: "A".to_owned(),
                position: 0,
            },
            PinnedFolder {
                path: "folder-b".to_owned(),
                display_name: "B".to_owned(),
                position: 2,
            },
        ];
        let groups = vec![
            FolderGroup {
                id: 7,
                name: "Group".to_owned(),
                position: 1,
                paths: Vec::new(),
            },
            FolderGroup {
                id: 9,
                name: "Another group".to_owned(),
                position: 3,
                paths: Vec::new(),
            },
        ];

        assert_eq!(
            ordered_quick_access_items(&folders, &groups),
            [
                QuickAccessItem::PinnedFolder("folder-a".to_owned()),
                QuickAccessItem::FolderGroup(7),
                QuickAccessItem::PinnedFolder("folder-b".to_owned()),
                QuickAccessItem::FolderGroup(9),
            ]
        );
    }

    #[test]
    fn insertion_gaps_map_to_final_indices_for_mixed_items() {
        assert_eq!(destination_for_quick_access_gap(0, 0, 4), 0);
        assert_eq!(destination_for_quick_access_gap(0, 1, 4), 0);
        assert_eq!(destination_for_quick_access_gap(0, 2, 4), 1);
        assert_eq!(destination_for_quick_access_gap(2, 1, 4), 1);
        assert_eq!(destination_for_quick_access_gap(2, 3, 4), 2);
        assert_eq!(destination_for_quick_access_gap(2, 4, 4), 3);
        assert_eq!(destination_for_quick_access_gap(0, 99, 4), 3);
    }
}
