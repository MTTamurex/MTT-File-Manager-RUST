use super::*;
use crate::domain::file_tag::{normalize_tag_path_key, TagColor};

#[test]
fn grouping_covers_the_complete_listing_not_just_the_visible_metadata_cache() {
    let template = FileEntry::from_path("file.txt".into(), false);
    let items: Vec<_> = (0..9000)
        .map(|index| {
            let mut item = template.clone();
            item.name = format!("file_{index}.txt");
            item.path = item.name.clone().into();
            item
        })
        .collect();
    let tags = items
        .iter()
        .take(8999)
        .map(|item| (normalize_tag_path_key(&item.path), vec![1]))
        .collect();
    let request = GroupRequest {
        signature: 1,
        items: Arc::new(items),
        column: ColumnId::Tags,
        tags: Arc::new(tags),
        definitions: rustc_hash::FxHashMap::from_iter([(
            1,
            FileTag {
                id: 1,
                name: "Example".into(),
                color: TagColor::Red,
                position: 0,
            },
        )]),
        cancelled: Arc::new(AtomicBool::new(false)),
        ctx: egui::Context::default(),
    };
    let result = run(&request).unwrap();
    assert_eq!(result.assignments.len(), 8999);
    let projection = result.project(&request.items, request.column, false);
    assert_eq!(projection.sections.len(), 2);
    assert_eq!(projection.sections[0].item_indices.len(), 8999);
    assert_eq!(projection.sections[1].item_indices.as_ref(), [8999]);
    request.cancelled.store(true, AtomicOrdering::Release);
    assert!(run(&request).is_none());
}
