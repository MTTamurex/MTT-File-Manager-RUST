//! Explicit, short-lived requests to reveal a folder without following navigation.
use super::SidebarTreeState;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

mod undo;
pub(crate) use undo::RevealOwner;
pub(super) use undo::RevealUndo;

pub(super) struct RevealRequest {
    owner: RevealOwner,
    root: PathBuf,
    target: PathBuf,
    started: Instant,
}

pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    let mut left = a.components();
    let mut right = b.components();
    loop {
        match (left.next(), right.next()) {
            (None, None) => return true,
            (Some(a), Some(b))
                if a.as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy()) => {}
            _ => return false,
        }
    }
}

pub(crate) fn contains_path(root: &Path, target: &Path) -> bool {
    target.ancestors().any(|ancestor| same_path(root, ancestor))
}

impl SidebarTreeState {
    pub(crate) fn cancel_reveal(&self) {
        self.reveal.borrow_mut().take();
    }

    pub(super) fn cancel_reveal_under(&self, path: &Path) {
        if self
            .reveal
            .borrow()
            .as_ref()
            .is_some_and(|request| contains_path(path, &request.target))
        {
            self.cancel_reveal();
        }
    }

    pub(super) fn cancel_reveal_on_load_failure(&self, path: &Path) {
        if self.reveal.borrow().as_ref().is_some_and(|request| {
            !same_path(path, &request.target) && contains_path(path, &request.target)
        }) {
            self.cancel_reveal();
        }
    }

    pub(crate) fn request_tab_reveal(&mut self, owner: RevealOwner, root: &Path, target: &Path) {
        if !contains_path(root, target) {
            return;
        }
        self.set_reveal_sidebar_workspace(owner.sidebar_workspace);
        self.reveal_undo
            .insert(owner, RevealUndo::new(root, target, self.expanded.clone()));
        *self.reveal.borrow_mut() = Some(RevealRequest {
            owner,
            root: root.to_path_buf(),
            target: target.to_path_buf(),
            started: Instant::now(),
        });
        self.advance_reveal();
    }

    #[cfg(test)]
    pub(crate) fn request_reveal(&mut self, root: &Path, target: &Path) {
        self.request_tab_reveal(RevealOwner::workspace(0), root, target);
    }

    pub(super) fn advance_reveal(&mut self) {
        let Some((owner, mut row, target)) = self
            .reveal
            .borrow()
            .as_ref()
            .filter(|request| request.started.elapsed() <= Duration::from_secs(10))
            .map(|request| (request.owner, request.root.clone(), request.target.clone()))
        else {
            return;
        };
        // Walk materialized rows, preserving their actual spelling on Windows.
        // Load one level at a time, rather than scheduling I/O for the whole path.
        while !same_path(&row, &target) {
            if self.expanded.insert(row.clone()) {
                if let Some(undo) = self.reveal_undo.get_mut(&owner) {
                    undo.opened.insert(row.clone());
                }
            }
            if !self.children.contains_key(&row) && !self.loading.contains(&row) {
                self.load_children(&row);
            }
            let next = self.children.get(&row).and_then(|children| {
                children
                    .iter()
                    .find(|node| contains_path(&node.path, &target))
                    .map(|node| node.path.clone())
            });
            let Some(next) = next else { break };
            row = next;
        }
    }

    /// Consume only when the requested row has materialized in the chosen tree.
    pub(crate) fn take_reveal_row(&self, root: &Path, row: &Path) -> bool {
        let mut pending = self.reveal.borrow_mut();
        if pending.as_ref().is_some_and(|request| {
            request.started.elapsed() <= Duration::from_secs(10)
                && same_path(&request.root, root)
                && same_path(&request.target, row)
        }) {
            pending.take();
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::directory_cache::DirectoryCache;
    use std::sync::Arc;

    #[test]
    fn matching_uses_components_not_string_prefixes() {
        assert!(contains_path(
            Path::new("C:\\Folder"),
            Path::new("c:\\folder\\Child")
        ));
        assert!(!contains_path(
            Path::new("C:\\Folder"),
            Path::new("C:\\FolderOther")
        ));
        assert!(same_path(Path::new("C:\\Folder\\"), Path::new("c:/folder")));
    }

    #[test]
    fn reveal_expands_only_ancestors_and_preserves_other_branches() {
        let mut tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
        let root = PathBuf::from("C:\\");
        let folder = root.join("Folder");
        let target = folder.join("Child");
        let unrelated = root.join("Other");
        let node = |path: PathBuf| super::super::FolderNode {
            path,
            name: "Folder".into(),
            has_subfolders: None,
            is_hidden: false,
        };
        tree.children
            .insert(root.clone(), vec![node(folder.clone())]);
        tree.children
            .insert(folder.clone(), vec![node(target.clone())]);
        tree.expanded.insert(unrelated.clone());
        tree.request_reveal(&root, &target);
        assert!(tree.is_expanded(&root));
        assert!(tree.is_expanded(&folder));
        assert!(tree.is_expanded(&unrelated));
        assert!(!tree.is_expanded(&target));
        assert!(!tree.take_reveal_row(&folder, &target));
        assert!(tree.take_reveal_row(&root, &target));
        assert!(!tree.take_reveal_row(&root, &target));
    }

    #[test]
    fn navigation_and_tab_restore_cancel_pending_reveal() {
        let mut tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
        let root = Path::new("C:\\");
        tree.request_reveal(root, root);
        tree.retain_tab_reveals(|_, path| same_path(path, Path::new("D:\\")));
        assert!(!tree.take_reveal_row(root, root));
        tree.request_reveal(root, root);
        tree.restore_expanded(Default::default(), 0.0);
        assert!(!tree.take_reveal_row(root, root));
    }

    #[test]
    fn reveal_continues_after_children_arrive_and_respects_manual_collapse() {
        let mut tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
        let root = PathBuf::from("C:\\");
        let folder = root.join("Folder");
        let target = folder.join("Child");
        // Simulate an in-flight root request without touching a real filesystem.
        tree.loading.insert(root.clone());
        tree.request_reveal(&root, &target);
        assert!(tree.is_expanded(&root));
        assert!(!tree.is_expanded(&folder));
        tree.loading.insert(folder.clone());
        tree.tx
            .send(super::super::LoadResult {
                parent: root.clone(),
                children: Some(vec![super::super::FolderNode {
                    path: folder.clone(),
                    name: "Folder".into(),
                    has_subfolders: None,
                    is_hidden: false,
                }]),
            })
            .unwrap();
        assert!(tree.poll_loaded());
        assert!(tree.is_expanded(&folder));
        assert!(!tree.is_expanded(&target));
        tree.toggle_expand(&root);
        tree.advance_reveal();
        assert!(!tree.is_expanded(&root));
        assert!(!tree.take_reveal_row(&root, &target));
    }

    #[test]
    fn unrelated_io_failure_does_not_cancel_reveal() {
        let mut tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
        let root = Path::new("C:\\");
        let unrelated = PathBuf::from("D:\\");
        tree.loading.insert(unrelated.clone());
        tree.request_reveal(root, root);
        tree.tx
            .send(super::super::LoadResult {
                parent: unrelated,
                children: None,
            })
            .unwrap();
        tree.poll_loaded();
        assert!(tree.take_reveal_row(root, root));
    }

    #[test]
    fn removing_the_requested_drive_cancels_reveal() {
        let mut tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
        let root = Path::new("C:\\");
        tree.request_reveal(root, root);
        tree.clear_drive(root);
        assert!(!tree.take_reveal_row(root, root));
    }

    #[test]
    fn failed_ancestor_is_not_reopened_or_retried() {
        let mut tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
        let root = Path::new("C:\\");
        let target = root.join("Folder");
        tree.loading.insert(root.into());
        tree.request_reveal(root, &target);
        tree.tx
            .send(super::super::LoadResult {
                parent: root.into(),
                children: None,
            })
            .unwrap();
        tree.poll_loaded();
        assert!(!tree.is_expanded(root));
        assert!(!tree.is_loading(root));
        assert!(tree.reveal.borrow().is_none());
    }

    #[test]
    fn navigation_is_validated_before_background_results_expand_more_nodes() {
        let mut tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
        let root = Path::new("C:\\");
        let folder = root.join("Folder");
        let target = folder.join("Child");
        tree.loading.insert(root.into());
        tree.request_reveal(root, &target);
        tree.tx
            .send(super::super::LoadResult {
                parent: root.into(),
                children: Some(vec![super::super::FolderNode {
                    path: folder.clone(),
                    name: "Folder".into(),
                    has_subfolders: None,
                    is_hidden: false,
                }]),
            })
            .unwrap();
        tree.retain_tab_reveals(|_, path| same_path(path, Path::new("D:\\")));
        tree.poll_loaded();
        assert!(!tree.is_expanded(&folder));
        assert!(!tree.is_loading(&folder));
    }

    #[test]
    fn expired_request_does_not_scroll_when_a_late_row_arrives() {
        let mut tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
        let root = Path::new("C:\\");
        tree.request_reveal(root, root);
        tree.reveal.borrow_mut().as_mut().unwrap().started =
            Instant::now() - Duration::from_secs(11);
        assert!(!tree.take_reveal_row(root, root));
    }
}
