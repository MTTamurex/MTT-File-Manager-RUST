use super::{search_page, IndexState, VolumeIndex};
use crate::volume_indices::handle_from;

#[test]
fn masks_and_alternatives_preserve_pagination_and_totals() {
    let mut index = VolumeIndex::empty('C');
    index.state = IndexState::Ready;
    for (offset, name) in ["file.EXE", "file.com", "file.exe.bak", "fileé.exe"]
        .iter()
        .enumerate()
    {
        assert!(index.insert_record(10 + offset as u64, name, 5, false, false));
    }
    let handles = [handle_from(index)];
    for (query, expected) in [
        ("*.exe;*.com", vec!["file.EXE", "file.com", "fileé.exe"]),
        (
            "*.exe;file*",
            vec!["file.EXE", "file.com", "file.exe.bak", "fileé.exe"],
        ),
        ("file?.exe", vec!["fileé.exe"]),
        ("; ;", vec![]),
    ] {
        let all = search_page(&handles, query, 0, 100);
        assert_eq!(all.total_matches, Some(expected.len()));
        assert!(!all.has_more);
        assert_eq!(
            all.items
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        for offset in 0..=expected.len() {
            let page = search_page(&handles, query, offset, 1);
            assert_eq!(
                page.items
                    .iter()
                    .map(|item| item.name.as_str())
                    .collect::<Vec<_>>(),
                expected
                    .iter()
                    .copied()
                    .skip(offset)
                    .take(1)
                    .collect::<Vec<_>>()
            );
            assert_eq!(page.has_more, offset + 1 < expected.len());
        }
    }
}
