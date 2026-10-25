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
    let filtered = filter_items_opt_with_tags(&items, "*.exe;*.com", Some(7), &tags).unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].name, "file.com");
}

#[test]
fn text_queries_keep_the_existing_behavior() {
    let items = [entry("final-report.txt"), entry("report.txt")];
    assert!(filter_items_opt(&items, " \t").is_none());
    assert!(filter_items_opt_with_tags(&items, "", None, &FxHashMap::default()).is_none());
    assert_eq!(filter_items(&items, "REPORT final").len(), 1);
}
