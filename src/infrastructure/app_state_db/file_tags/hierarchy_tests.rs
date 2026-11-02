use super::*;

#[test]
fn create_tag_persists_the_parent_and_normalizes_orphans() {
    let db = AppStateDb::new_in_memory().unwrap();
    let parent = db.create_tag("Fotos", TagColor::Blue, None).unwrap();
    let child = db
        .create_tag("Férias 2025", TagColor::Green, Some(parent))
        .unwrap();
    let orphan = db.create_tag("Órfã", TagColor::Red, Some(999)).unwrap();

    let tags = db.get_all_tags();
    let child_tag = tags.iter().find(|tag| tag.id == child).unwrap();
    assert_eq!(child_tag.parent_id, Some(parent));
    // A stale parent link (tag 999 never existed) is normalized to a root.
    let orphan_tag = tags.iter().find(|tag| tag.id == orphan).unwrap();
    assert_eq!(orphan_tag.parent_id, None);
}

#[test]
fn update_tag_parent_moves_tags_between_parents() {
    let db = AppStateDb::new_in_memory().unwrap();
    let first = db.create_tag("Fotos", TagColor::Blue, None).unwrap();
    let second = db.create_tag("Músicas", TagColor::Red, None).unwrap();
    let child = db
        .create_tag("Férias", TagColor::Green, Some(first))
        .unwrap();

    assert!(db.update_tag_parent(child, Some(second)));
    let tags = db.get_all_tags();
    let child_tag = tags.iter().find(|tag| tag.id == child).unwrap();
    assert_eq!(child_tag.parent_id, Some(second));

    assert!(db.update_tag_parent(child, None));
    let tags = db.get_all_tags();
    let child_tag = tags.iter().find(|tag| tag.id == child).unwrap();
    assert_eq!(child_tag.parent_id, None);
}

#[test]
fn delete_tag_subtree_removes_descendants_and_their_assignments() {
    let db = AppStateDb::new_in_memory().unwrap();
    let root = db.create_tag("Fotos", TagColor::Blue, None).unwrap();
    let child = db
        .create_tag("Férias", TagColor::Green, Some(root))
        .unwrap();
    let grandchild = db
        .create_tag("2025", TagColor::Yellow, Some(child))
        .unwrap();
    let survivor = db.create_tag("Músicas", TagColor::Red, None).unwrap();

    let root_only = PathBuf::from(r"C:\Library\root.jpg");
    let subtree_only = PathBuf::from(r"C:\Library\vacation.jpg");
    let shared = PathBuf::from(r"C:\Library\both.jpg");
    let survivor_path = PathBuf::from(r"C:\Library\song.mp3");
    for (path, tag_id) in [
        (&root_only, root),
        (&shared, root),
        (&subtree_only, grandchild),
        (&shared, grandchild),
        (&survivor_path, survivor),
    ] {
        assert!(db.assign_tags_batch(std::slice::from_ref(path), &[tag_id]));
    }

    // The DB layer deletes exactly the given ids; the operations layer
    // resolves the subtree before calling it.
    let removed = db.delete_tag_subtree(&[root, child, grandchild]);
    assert_eq!(removed, 3 + 4); // 3 tag rows + 4 assignment rows

    let tags = db.get_all_tags();
    assert!(tags.iter().any(|tag| tag.id == survivor));
    assert!(!tags.iter().any(|tag| tag.id == root));
    assert!(!tags.iter().any(|tag| tag.id == child));
    assert!(!tags.iter().any(|tag| tag.id == grandchild));

    assert_eq!(db.get_tag_assignment_paths(survivor), vec![survivor_path]);
    assert!(db.get_tag_assignment_paths(root).is_empty());
    assert!(db.get_tag_assignment_paths(grandchild).is_empty());
}

#[test]
fn implicit_parent_assign_and_unassign_keep_the_invariant() {
    use crate::domain::tag_tree::{tags_to_assign, tags_to_remove};

    let db = AppStateDb::new_in_memory().unwrap();
    let root = db.create_tag("Fotos", TagColor::Blue, None).unwrap();
    let trip = db
        .create_tag("Férias", TagColor::Green, Some(root))
        .unwrap();
    let year = db.create_tag("2025", TagColor::Red, Some(trip)).unwrap();
    let sibling = db
        .create_tag("Passeio", TagColor::Teal, Some(root))
        .unwrap();
    let path = PathBuf::from(r"C:\Library\photo.jpg");

    let defs = || -> FxHashMap<i64, FileTag> {
        db.get_all_tags()
            .into_iter()
            .map(|tag| (tag.id, tag))
            .collect()
    };
    let tags_on_path = |db: &AppStateDb| -> FxHashSet<i64> {
        db.get_all_tag_assignments()
            .get(&path)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect()
    };

    // Marking "2025" writes the whole chain: 2025, Férias, Fotos.
    assert!(db.assign_tags_batch(std::slice::from_ref(&path), &tags_to_assign(&defs(), year)));
    assert_eq!(tags_on_path(&db), [root, trip, year].into_iter().collect());

    // Also mark a sibling sub-tag, then uncheck "Férias": its subtree
    // (Férias, 2025) goes, while the shared parent "Fotos" stays because
    // "Passeio" still needs it.
    assert!(db.assign_tags_batch(
        std::slice::from_ref(&path),
        &tags_to_assign(&defs(), sibling)
    ));
    assert!(db.unassign_tags_batch(std::slice::from_ref(&path), &tags_to_remove(&defs(), trip)));
    assert_eq!(tags_on_path(&db), [root, sibling].into_iter().collect());

    // Unchecking the root removes what remains of its subtree on this file.
    assert!(db.unassign_tags_batch(std::slice::from_ref(&path), &tags_to_remove(&defs(), root)));
    assert!(tags_on_path(&db).is_empty());
}

#[test]
fn multi_tag_path_query_deduplicates_paths_shared_by_the_subtree() {
    let db = AppStateDb::new_in_memory().unwrap();
    let root = db.create_tag("Fotos", TagColor::Blue, None).unwrap();
    let child = db
        .create_tag("Férias", TagColor::Green, Some(root))
        .unwrap();

    let shared = PathBuf::from(r"C:\Library\both.jpg");
    let child_only = PathBuf::from(r"C:\Library\vacation.jpg");
    for (path, tag_id) in [(&shared, root), (&child_only, child), (&shared, child)] {
        assert!(db.assign_tags_batch(std::slice::from_ref(path), &[tag_id]));
    }

    let paths = db.get_tag_assignment_paths_after_for_tags(&[root, child], None, 10);
    // One row per distinct path, ordered; `shared` appears exactly once even
    // though both subtree tags point at it.
    assert_eq!(paths, vec![shared.clone(), child_only.clone()]);

    let page = db.get_tag_assignment_paths_after_for_tags(&[root, child], Some(&shared), 10);
    assert_eq!(page, vec![child_only]);

    let limited = db.get_tag_assignment_paths_after_for_tags(&[root, child], None, 1);
    assert_eq!(limited, vec![shared]);
}
