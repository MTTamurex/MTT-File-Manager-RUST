use super::*;
use crate::domain::file_tag::TagColor;
use std::path::PathBuf;

fn definitions() -> FxHashMap<i64, FileTag> {
    [
        (1, "Category A", None),
        (2, "Shared name", Some(1)),
        (3, "Nested tag", Some(2)),
        (4, "Category B", None),
        (5, "Shared name", Some(4)),
    ]
    .into_iter()
    .map(|(id, name, parent_id)| {
        (
            id,
            FileTag {
                id,
                name: name.into(),
                color: TagColor::Blue,
                position: id,
                parent_id,
            },
        )
    })
    .collect()
}

#[test]
fn collapsed_branches_hide_descendants_but_keep_siblings() {
    assert_eq!(
        visible_tags(
            &definitions(),
            &[1, 2, 3, 4, 5],
            "",
            &[1].into_iter().collect()
        ),
        vec![1, 4, 5]
    );
}

#[test]
fn search_reveals_matches_and_ancestors_without_changing_collapse_state() {
    let collapsed = [1, 2].into_iter().collect();
    assert_eq!(
        visible_tags(&definitions(), &[1, 2, 3, 4, 5], "  NESTED ", &collapsed),
        vec![1, 2, 3]
    );
    assert_eq!(
        visible_tags(&definitions(), &[1, 2, 3, 4, 5], "", &collapsed),
        vec![1, 4, 5]
    );
}

#[test]
fn duplicate_names_keep_both_branches_and_distinct_breadcrumbs() {
    let defs = definitions();
    assert_eq!(
        visible_tags(&defs, &[1, 2, 3, 4, 5], "shared", &FxHashSet::default()),
        vec![1, 2, 4, 5]
    );
    let labels = tag_labels(&defs);
    assert_eq!(labels[&2], "Category A / Shared name");
    assert_eq!(labels[&5], "Category B / Shared name");
}

#[test]
fn rules_search_path_or_complete_tag_hierarchy_including_disabled_rules() {
    let rules = vec![
        AutoTagRule {
            id: 10,
            folder_path: PathBuf::from(r"C:\Library\Folder A"),
            tag_id: 3,
            enabled: true,
            include_subfolders: true,
            position: 0,
        },
        AutoTagRule {
            id: 11,
            folder_path: PathBuf::from(r"D:\Library\Folder B"),
            tag_id: 5,
            enabled: false,
            include_subfolders: false,
            position: 1,
        },
    ];
    let labels = tag_labels(&definitions());
    assert_eq!(visible_rules(&rules, &labels, "folder a"), vec![10]);
    assert_eq!(visible_rules(&rules, &labels, "CATEGORY B"), vec![11]);
    assert_eq!(visible_rules(&rules, &labels, "shared"), vec![10, 11]);
    assert!(visible_rules(&rules, &labels, "missing").is_empty());
}

#[test]
fn stale_selection_is_cleared_after_deletion_without_touching_search() {
    let mut list = ListState {
        selected: Some(2),
        search: "query".into(),
        ..Default::default()
    };
    list.reconcile(|_| false);
    assert_eq!(list.selected, None);
    assert_eq!(list.search, "query");
}

#[test]
fn tab_state_preserves_independent_search_and_selection() {
    let mut browser = BrowserState::default();
    browser.tags.search = "tag".into();
    browser.tags.select(3);
    browser.tab = Tab::AutoTag;
    browser.rules.search = "folder".into();
    browser.rules.select(10);
    browser.tab = Tab::Tags;
    assert_eq!(browser.tags.search, "tag");
    assert_eq!(browser.tags.selected, Some(3));
    assert_eq!(browser.rules.search, "folder");
    assert_eq!(browser.rules.selected, Some(10));
}

#[test]
fn creation_does_not_replace_or_clear_either_selected_editor() {
    let mut browser = BrowserState::default();
    browser.tags.select(2);
    browser.tags.search = "tag query".into();
    browser.rules.select(10);
    browser.rules.search = "rule query".into();
    for tab in [Tab::Tags, Tab::AutoTag] {
        browser.creation = Some(tab);
        assert_eq!(browser.tags.selected, Some(2));
        assert_eq!(browser.rules.selected, Some(10));
        browser.creation = None;
        assert_eq!(browser.tags.search, "tag query");
        assert_eq!(browser.rules.search, "rule query");
    }
}

#[test]
fn searching_cancels_pending_keyboard_focus_without_changing_selection() {
    let mut list = ListState {
        selected: Some(2),
        focus_selected: true,
        ..Default::default()
    };
    list.search_changed();
    assert!(list.reset_scroll);
    assert!(!list.focus_selected);
    assert_eq!(list.selected, Some(2));
    list.focus_selected = true;
    list.reconcile(|_| false);
    assert!(!list.focus_selected);
}

#[test]
fn keyboard_navigation_is_bounded_to_visible_results() {
    assert_eq!(adjacent(&[1, 4, 5], Some(1), true), Some(4));
    assert_eq!(adjacent(&[1, 4, 5], Some(5), true), Some(5));
    assert_eq!(adjacent(&[1, 4, 5], Some(1), false), Some(1));
    assert_eq!(adjacent(&[1, 4, 5], Some(3), true), Some(1));
    assert_eq!(adjacent(&[], None, true), None);
}

#[test]
fn malformed_parent_cycles_do_not_hang_search_or_labels() {
    let mut defs = definitions();
    defs.get_mut(&1).unwrap().parent_id = Some(3);
    assert_eq!(
        visible_tags(&defs, &[1, 2, 3], "nested", &FxHashSet::default()),
        vec![1, 2, 3]
    );
    assert!(!tag_labels(&defs)[&3].is_empty());
}
