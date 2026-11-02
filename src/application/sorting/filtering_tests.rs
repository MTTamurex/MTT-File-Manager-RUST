use super::*;
use crate::domain::file_entry::SyncStatus;
use std::path::PathBuf;

fn entry(name: &str) -> FileEntry {
    FileEntry {
        path: PathBuf::from(name),
        name: name.to_owned(),
        is_dir: false,
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

#[test]
fn masks_and_alternatives_work_with_both_filter_apis() {
    let items = [
        entry("file.EXE"),
        entry("file.com"),
        entry("file.exe.bak"),
        entry("fileé.exe"),
    ];
    for (query, expected) in [
        ("*.exe;*.com", vec!["file.EXE", "file.com", "fileé.exe"]),
        ("file?.exe", vec!["fileé.exe"]),
        ("file *.exe", vec!["file.EXE", "fileé.exe"]),
        ("; ;", vec![]),
    ] {
        for filtered in [
            filter_items_opt(&items, query).unwrap(),
            filter_items_opt_with_tags(&items, query, None, &FxHashMap::default()).unwrap(),
        ] {
            assert_eq!(
                filtered
                    .iter()
                    .map(|item| item.name.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
}

#[test]
fn alternatives_do_not_bypass_tag_filters() {
    let items = [entry("file.exe"), entry("file.com"), entry("file.txt")];
    let mut tags = FxHashMap::default();
    for item in &items[1..] {
        tags.insert(file_tag::normalize_tag_path_key(&item.path), vec![7]);
    }
    let filtered = filter_items_opt_with_tags(&items, "*.exe;*.com", Some(&[7]), &tags).unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].name, "file.com");
}

#[test]
fn tag_filter_matches_any_id_in_the_resolved_set() {
    let items = [entry("file.exe"), entry("file.com"), entry("file.txt")];
    let mut tags = FxHashMap::default();
    tags.insert(file_tag::normalize_tag_path_key(&items[0].path), vec![7]);
    tags.insert(file_tag::normalize_tag_path_key(&items[1].path), vec![8]);
    let filtered = filter_items_opt_with_tags(&items, "", Some(&[7, 8]), &tags).unwrap();
    assert_eq!(filtered.len(), 2);
    // An empty resolved set matches nothing — a tag view never shows unfiltered items.
    let empty = filter_items_opt_with_tags(&items, "", Some(&[]), &tags).unwrap();
    assert!(empty.is_empty());
}

#[test]
fn parent_tag_view_shows_descendants_but_not_siblings() {
    use crate::domain::file_tag::{FileTag, TagColor};
    use crate::domain::tag_tree::subtree_ids;

    let make = |id: i64, parent: Option<i64>| FileTag {
        id,
        name: format!("tag{id}"),
        color: TagColor::Blue,
        position: id,
        parent_id: parent,
    };
    // 1 "Fotos" ── 2 "Férias" ── 3 "2025"; 4 "Músicas" is a sibling root.
    let defs: FxHashMap<i64, FileTag> = [
        make(1, None),
        make(2, Some(1)),
        make(3, Some(2)),
        make(4, None),
    ]
    .into_iter()
    .map(|tag| (tag.id, tag))
    .collect();

    let items = [
        entry("parent.jpg"),
        entry("child.jpg"),
        entry("grandchild.jpg"),
        entry("sibling.mp3"),
    ];
    let mut tags = FxHashMap::default();
    tags.insert(file_tag::normalize_tag_path_key(&items[0].path), vec![1]);
    tags.insert(file_tag::normalize_tag_path_key(&items[1].path), vec![2]);
    tags.insert(file_tag::normalize_tag_path_key(&items[2].path), vec![3]);
    tags.insert(file_tag::normalize_tag_path_key(&items[3].path), vec![4]);

    // Viewing the "Fotos" parent resolves to its whole subtree.
    let resolved = subtree_ids(&defs, 1);
    let filtered = filter_items_opt_with_tags(&items, "", Some(&resolved), &tags).unwrap();
    let names: Vec<&str> = filtered.iter().map(|item| item.name.as_str()).collect();
    assert_eq!(names, vec!["parent.jpg", "child.jpg", "grandchild.jpg"]);
}

#[test]
fn text_queries_keep_the_existing_behavior() {
    let items = [entry("final-report.txt"), entry("report.txt")];
    assert!(filter_items_opt(&items, " \t").is_none());
    assert!(filter_items_opt_with_tags(&items, "", None, &FxHashMap::default()).is_none());
    assert_eq!(filter_items(&items, "REPORT final").len(), 1);
}
