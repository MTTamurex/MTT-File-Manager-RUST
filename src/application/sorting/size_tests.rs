use super::*;
use std::collections::HashMap;
use std::path::PathBuf;

fn entry(name: &str, is_dir: bool, size: u64) -> FileEntry {
    FileEntry {
        path: PathBuf::from(name),
        name: name.into(),
        is_dir,
        size,
        modified: 0,
        created: None,
        folder_cover: None,
        drive_info: None,
        sync_status: crate::domain::file_entry::SyncStatus::None,
        is_hidden: false,
        recycle_bin: None,
    }
}

#[test]
fn calculated_sizes_sort_both_directions_without_changing_metadata() {
    let mut items = vec![entry("large", true, 0), entry("small", true, 0)];
    let sizes = HashMap::from([(PathBuf::from("large"), 500), (PathBuf::from("small"), 10)]);
    for (descending, expected) in [(false, "small"), (true, "large")] {
        sort_items_with_folder_sizes(
            &mut items,
            SortMode::Size,
            descending,
            FoldersPosition::First,
            &sizes,
        );
        assert_eq!(items[0].name, expected);
        assert!(items.iter().all(|item| item.size == 0));
    }
}

#[test]
fn metadata_only_sort_keeps_existing_directory_sizes() {
    let mut items = vec![entry("small", true, 10), entry("large", true, 500)];
    sort_items(&mut items, SortMode::Size, true, FoldersPosition::First);
    assert_eq!(items[0].name, "large");
}

#[test]
fn parallel_sort_uses_calculated_sizes() {
    let mut items: Vec<_> = (0..6000)
        .map(|index| entry(&format!("folder-{index}"), true, 0))
        .collect();
    let sizes = items
        .iter()
        .enumerate()
        .map(|(index, item)| (item.path.clone(), index as u64))
        .collect();
    sort_items_with_folder_sizes(
        &mut items,
        SortMode::Size,
        true,
        FoldersPosition::First,
        &sizes,
    );
    assert_eq!(items.first().unwrap().name, "folder-5999");
    assert_eq!(items.last().unwrap().name, "folder-0");
}

#[test]
fn pending_sizes_follow_known_sizes_including_empty_folders() {
    let sizes = HashMap::from([(PathBuf::from("empty"), 0)]);
    for descending in [false, true] {
        let mut items = vec![entry("pending", true, 0), entry("empty", true, 0)];
        sort_items_with_folder_sizes(
            &mut items,
            SortMode::Size,
            descending,
            FoldersPosition::Mixed,
            &sizes,
        );
        assert_eq!(items[0].name, "empty");
        assert_eq!(items[1].name, "pending");
    }
}

#[test]
fn folder_position_is_independent_of_size_direction() {
    let sizes = HashMap::from([(PathBuf::from("folder"), 100)]);
    for descending in [false, true] {
        for (position, expected) in [
            (FoldersPosition::First, "folder"),
            (FoldersPosition::Last, "file.txt"),
        ] {
            let mut items = vec![entry("file.txt", false, 50), entry("folder", true, 0)];
            sort_items_with_folder_sizes(&mut items, SortMode::Size, descending, position, &sizes);
            assert_eq!(items[0].name, expected);
        }
    }
}

#[test]
fn filtered_items_and_archives_use_the_correct_sizes() {
    let items = vec![
        entry("folder-a", true, 0),
        entry("folder-b", true, 0),
        entry("archive.zip", true, 50),
    ];
    let sizes = HashMap::from([
        (PathBuf::from("folder-a"), 100),
        (PathBuf::from("folder-b"), 10),
    ]);
    let mut filtered = filter_items(&items, "folder");
    sort_items_with_folder_sizes(
        &mut filtered,
        SortMode::Size,
        false,
        FoldersPosition::Mixed,
        &sizes,
    );
    assert_eq!(filtered[0].name, "folder-b");
    let mut items = items;
    sort_items_with_folder_sizes(
        &mut items,
        SortMode::Size,
        false,
        FoldersPosition::Mixed,
        &sizes,
    );
    assert_eq!(
        items
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        ["folder-b", "archive.zip", "folder-a"]
    );
}
