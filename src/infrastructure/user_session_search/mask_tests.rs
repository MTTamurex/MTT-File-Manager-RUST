use super::tests::{empty_index, test_volume};

#[test]
fn masks_keep_counts_pagination_and_index_parity() {
    let names = [
        "file.EXE",
        "file.com",
        "file.exe.bak",
        "fileé.exe",
        "other.txt",
        "a",
        "résumé.TXT",
    ];
    for with_index in [false, true] {
        let mut index = empty_index();
        index.volumes.insert('C', test_volume(&names, with_index));
        for (query, expected) in [
            ("*.exe;*.com", vec!["file.EXE", "file.com", "fileé.exe"]),
            (
                "*.exe;file*",
                vec!["file.EXE", "file.com", "file.exe.bak", "fileé.exe"],
            ),
            ("file?.exe", vec!["fileé.exe"]),
            ("*.missing;*.com", vec!["file.com"]),
            ("*.exe;?", vec!["file.EXE", "fileé.exe", "a"]),
            ("r?sum?.txt", vec!["résumé.TXT"]),
            ("*.exe;*", names.to_vec()),
            ("; ;", vec![]),
        ] {
            assert_eq!(
                index.count_matches(query) as usize,
                expected.len(),
                "{query}"
            );
            let (all, has_more) = index.search_page(query, 0, 100);
            assert!(!has_more);
            assert_eq!(
                all.iter()
                    .map(|item| item.name.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            for offset in 0..=expected.len() {
                let (page, has_more) = index.search_page(query, offset, 1);
                assert_eq!(
                    page.iter()
                        .map(|item| item.name.as_str())
                        .collect::<Vec<_>>(),
                    expected
                        .iter()
                        .copied()
                        .skip(offset)
                        .take(1)
                        .collect::<Vec<_>>()
                );
                assert_eq!(has_more, offset + 1 < expected.len());
            }
        }
    }
}

#[test]
fn masks_exclude_deleted_paths() {
    let mut index = empty_index();
    let mut volume = test_volume(&["file.exe", "other.com"], true);
    volume.live_paths.remove(&volume.items[0].path_key);
    index.volumes.insert('C', volume);
    assert_eq!(index.count_matches("*.exe;*.com"), 1);
    assert_eq!(index.search("*.exe;*.com", 10)[0].name, "other.com");
}
