//! Undo records belong to stable tab identities, never to tab strip positions.
use super::{contains_path, same_path, SidebarTreeState};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RevealOwner {
    pub workspace: usize,
    pub panel_tab: Option<usize>,
    pub sidebar_workspace: usize,
}

impl RevealOwner {
    pub(crate) fn workspace(workspace: usize) -> Self {
        Self {
            workspace,
            panel_tab: None,
            sidebar_workspace: workspace,
        }
    }

    pub(crate) fn panel(workspace: usize, tab: usize) -> Self {
        Self {
            workspace,
            panel_tab: Some(tab),
            sidebar_workspace: workspace,
        }
    }

    pub(crate) fn in_sidebar(mut self, workspace: usize) -> Self {
        self.sidebar_workspace = workspace;
        self
    }
}

pub(in super::super) struct RevealUndo {
    pub(super) target: PathBuf,
    root: PathBuf,
    pub(in super::super) opened: HashSet<PathBuf>,
    existing: HashSet<PathBuf>,
}

impl RevealUndo {
    pub(super) fn new(root: &Path, target: &Path, existing: HashSet<PathBuf>) -> Self {
        Self {
            root: root.into(),
            target: target.into(),
            opened: Default::default(),
            existing,
        }
    }
}

impl SidebarTreeState {
    pub(crate) fn set_reveal_sidebar_workspace(&mut self, workspace: usize) {
        self.reveal_sidebar_workspace = Some(workspace);
    }

    pub(crate) fn tab_tree_action(
        &self,
        owner: RevealOwner,
        path: &str,
    ) -> Option<crate::tabs::TabAction> {
        if path.is_empty() || crate::domain::special_paths::is_virtual_path(path) {
            return None;
        }
        Some(if self.can_undo_tab_reveal(owner, Path::new(path)) {
            crate::tabs::TabAction::CollapseFolderTree
        } else {
            crate::tabs::TabAction::ExpandFolderTree
        })
    }

    pub(crate) fn can_undo_tab_reveal(&self, owner: RevealOwner, target: &Path) -> bool {
        self.reveal_undo.get(&owner).is_some_and(|undo| {
            same_path(&undo.target, target)
                && (undo.opened.iter().any(|path| {
                    self.expanded.contains(path)
                        && path
                            .ancestors()
                            .skip(1)
                            .take_while(|ancestor| contains_path(&undo.root, ancestor))
                            .all(|ancestor| {
                                self.expanded
                                    .iter()
                                    .any(|expanded| same_path(expanded, ancestor))
                            })
                }) || self.reveal.borrow().as_ref().is_some_and(|request| {
                    request.owner == owner
                        && request.started.elapsed() <= std::time::Duration::from_secs(10)
                }))
        })
    }

    pub(crate) fn undo_tab_reveal(&mut self, owner: RevealOwner, target: &Path) {
        if !self.can_undo_tab_reveal(owner, target) {
            return;
        }
        if self
            .reveal
            .borrow()
            .as_ref()
            .is_some_and(|request| request.owner == owner)
        {
            self.cancel_reveal();
        }
        let Some(undo) = self.reveal_undo.remove(&owner) else {
            return;
        };
        let mut opened: Vec<_> = undo.opened.into_iter().collect();
        opened.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
        for path in opened {
            // A branch opened manually (or by another tab) after the action
            // still needs its ancestors. Never hide it as a side effect of undo.
            if !self.expanded.iter().any(|other| {
                !undo.existing.contains(other)
                    && !same_path(other, &path)
                    && contains_path(&path, other)
            }) {
                self.expanded.remove(&path);
            }
        }
        self.prune_collapsed_children_cache();
    }

    pub(crate) fn retain_tab_reveals(&mut self, valid: impl Fn(RevealOwner, &Path) -> bool) {
        self.reveal_undo
            .retain(|owner, undo| valid(*owner, &undo.target));
        if self.reveal.borrow().as_ref().is_some_and(|request| {
            !valid(request.owner, &request.target)
                || request.started.elapsed() > std::time::Duration::from_secs(10)
        }) {
            self.cancel_reveal();
        }
    }

    pub(in super::super) fn preserve_manual_expansion(&mut self, path: &Path) {
        for (owner, undo) in &mut self.reveal_undo {
            if Some(owner.sidebar_workspace) != self.reveal_sidebar_workspace {
                continue;
            }
            undo.opened.retain(|opened| !same_path(opened, path));
            // An old, hidden expansion explicitly changed by the user is no
            // longer just part of the pre-action baseline.
            undo.existing.retain(|existing| !same_path(existing, path));
        }
    }
}

#[cfg(test)]
mod tests;
