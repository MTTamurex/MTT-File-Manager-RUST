use super::*;
use crate::app::state::sidebar_tree_state::FolderNode;
use crate::infrastructure::directory_cache::DirectoryCache;
use crate::tabs::TabAction;
use std::sync::Arc;

fn node(path: &Path) -> FolderNode {
    FolderNode {
        path: path.into(),
        name: "Folder".into(),
        has_subfolders: None,
        is_hidden: false,
    }
}

fn tree() -> SidebarTreeState {
    let mut tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
    tree.children
        .insert("C:\\".into(), vec![node(Path::new("C:\\Folder"))]);
    tree.children.insert(
        "C:\\Folder".into(),
        vec![node(Path::new("C:\\Folder\\Child"))],
    );
    tree
}

#[test]
fn undo_only_removes_the_ancestors_opened_by_this_tab() {
    let mut tree = tree();
    let owner = RevealOwner::workspace(1);
    let target = Path::new("C:\\Folder\\Child");
    tree.expanded.insert("C:\\".into());
    tree.expanded.insert("D:\\".into());
    tree.request_tab_reveal(owner, Path::new("C:\\"), target);
    tree.take_reveal_row(Path::new("C:\\"), target);
    assert_eq!(
        tree.tab_tree_action(owner, &target.to_string_lossy()),
        Some(TabAction::CollapseFolderTree)
    );
    tree.undo_tab_reveal(owner, target);
    assert!(tree.is_expanded(Path::new("C:\\")));
    assert!(tree.is_expanded(Path::new("D:\\")));
    assert!(!tree.is_expanded(Path::new("C:\\Folder")));
    assert_eq!(
        tree.tab_tree_action(owner, &target.to_string_lossy()),
        Some(TabAction::ExpandFolderTree)
    );
}

#[test]
fn undo_restores_hidden_previously_expanded_descendants_without_closing_them() {
    let mut tree = tree();
    let owner = RevealOwner::workspace(1);
    let root = Path::new("C:\\");
    let target = Path::new("C:\\Folder\\Child");
    tree.expanded.insert(target.into());
    tree.request_tab_reveal(owner, root, target);
    tree.undo_tab_reveal(owner, target);
    assert!(!tree.is_expanded(root));
    assert!(!tree.is_expanded(Path::new("C:\\Folder")));
    assert!(tree.is_expanded(target));
}

#[test]
fn undo_preserves_branches_opened_manually_after_the_action() {
    let mut tree = tree();
    let owner = RevealOwner::workspace(1);
    let target = Path::new("C:\\Folder\\Child");
    tree.children
        .insert(target.into(), vec![node(&target.join("Subfolder"))]);
    tree.request_tab_reveal(owner, Path::new("C:\\"), target);
    tree.toggle_expand(target);
    tree.undo_tab_reveal(owner, target);
    assert!(tree.is_expanded(target));
    assert!(tree.is_expanded(Path::new("C:\\Folder")));
    assert!(tree.is_expanded(Path::new("C:\\")));
}

#[test]
fn a_manually_collapsed_ancestor_changes_the_menu_back_to_expand() {
    let mut tree = tree();
    let owner = RevealOwner::workspace(1);
    let root = Path::new("C:\\");
    let target = Path::new("C:\\Folder\\Child");
    tree.request_tab_reveal(owner, root, target);
    tree.take_reveal_row(root, target);
    tree.toggle_expand(root);
    assert_eq!(
        tree.tab_tree_action(owner, &target.to_string_lossy()),
        Some(TabAction::ExpandFolderTree)
    );
}

#[test]
fn undo_is_scoped_to_the_clicked_tab_and_visible_workspace() {
    let mut tree = tree();
    let owner = RevealOwner::workspace(2).in_sidebar(1);
    let root = Path::new("C:\\");
    let target = Path::new("C:\\Folder\\Child");
    tree.request_tab_reveal(owner, root, target);
    tree.take_reveal_row(root, target);
    assert!(tree.can_undo_tab_reveal(owner, target));
    assert!(!tree.can_undo_tab_reveal(RevealOwner::workspace(1), target));
    assert!(!tree.can_undo_tab_reveal(RevealOwner::workspace(2), target));
    assert!(!tree.can_undo_tab_reveal(owner, Path::new("D:\\")));
    tree.retain_tab_reveals(|_, _| false);
    assert!(!tree.can_undo_tab_reveal(owner, target));
}

#[test]
fn undo_during_loading_does_not_reopen_ancestors_when_results_arrive() {
    let mut tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
    let owner = RevealOwner::workspace(2).in_sidebar(1);
    let root = Path::new("C:\\");
    let target = Path::new("C:\\Folder");
    tree.loading.insert(root.into());
    tree.request_tab_reveal(owner, root, target);
    tree.undo_tab_reveal(owner, target);
    tree.tx
        .send(crate::app::state::sidebar_tree_state::LoadResult {
            parent: root.into(),
            children: Some(vec![node(target)]),
        })
        .unwrap();
    tree.poll_loaded();
    assert!(!tree.is_expanded(root));
    assert!(!tree.take_reveal_row(root, target));
}

#[test]
fn virtual_views_never_offer_tree_actions() {
    let tree = tree();
    for path in ["::computer::", "::recycle_bin::", "::tag::1", ""] {
        assert_eq!(tree.tab_tree_action(RevealOwner::workspace(1), path), None);
    }
}

#[test]
fn manual_changes_in_another_workspace_do_not_destroy_this_tabs_undo() {
    let mut tree = tree();
    let owner = RevealOwner::workspace(1);
    let root = Path::new("C:\\");
    let target = Path::new("C:\\Folder\\Child");
    tree.request_tab_reveal(owner, root, target);
    tree.take_reveal_row(root, target);
    let expanded = tree.snapshot_expanded();
    tree.set_reveal_sidebar_workspace(2);
    tree.toggle_expand(root);
    tree.set_reveal_sidebar_workspace(1);
    tree.restore_expanded(expanded, 0.0);
    tree.undo_tab_reveal(owner, target);
    assert!(!tree.is_expanded(root));
}

#[test]
fn a_preexisting_hidden_branch_manually_reopened_after_reveal_is_preserved() {
    let mut tree = tree();
    let owner = RevealOwner::workspace(1);
    let root = Path::new("C:\\");
    let target = Path::new("C:\\Folder\\Child");
    tree.expanded.insert(target.into());
    tree.loading.insert(target.into());
    tree.request_tab_reveal(owner, root, target);
    tree.toggle_expand(target);
    tree.toggle_expand(target);
    tree.undo_tab_reveal(owner, target);
    assert!(tree.is_expanded(root));
    assert!(tree.is_expanded(target));
}
