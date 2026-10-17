use super::{build_filtered_indices, matches_category};
use crate::app::global_search_state::{GlobalSearchCategory, GlobalSearchTagFilter};
use mtt_search_protocol::SearchResultItem;

fn result(name: &str, is_dir: bool) -> SearchResultItem {
    SearchResultItem {
        name: name.to_string(),
        full_path: format!(r"C:\Files\{name}"),
        is_dir,
        size: 1024,
    }
}

#[test]
fn applications_include_executables_installers_shortcuts_and_scripts() {
    for extension in [
        "exe", "msi", "msix", "appx", "com", "scr", "lnk", "bat", "cmd", "ps1",
    ] {
        for extension in [extension.to_string(), extension.to_ascii_uppercase()] {
            let item = result(&format!("file.{extension}"), false);
            assert!(
                matches_category(&item, GlobalSearchCategory::Applications),
                "Expected application extension: {extension}"
            );
        }
    }
}

#[test]
fn applications_exclude_directories_and_other_file_types() {
    for name in [
        "file.txt", "file.png", "file.mp4", "file.mp3", "file.dll", "file",
    ] {
        assert!(!matches_category(
            &result(name, false),
            GlobalSearchCategory::Applications
        ));
    }
    assert!(!matches_category(
        &result("folder.exe", true),
        GlobalSearchCategory::Applications
    ));
}

#[test]
fn applications_filter_preserves_size_and_tag_filters() {
    let results = vec![
        result("file.exe", false),
        result("file.lnk", false),
        result("file.ps1", false),
        result("file.txt", false),
    ];
    let mut assignments = rustc_hash::FxHashMap::default();
    assignments.insert(r"c:\files\file.lnk".to_string(), vec![1]);
    assignments.insert(r"c:\files\file.ps1".to_string(), vec![1]);
    let mut results = results;
    results[2].size = 4096;

    let filtered = build_filtered_indices(
        &results,
        GlobalSearchCategory::Applications,
        Some('C'),
        Some(512),
        Some(2048),
        None,
        None,
        &[],
        &GlobalSearchTagFilter::Selected(vec![1]),
        &assignments,
    );

    assert_eq!(filtered, vec![1]);
}
