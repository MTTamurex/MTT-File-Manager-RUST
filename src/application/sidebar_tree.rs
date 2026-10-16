//! Explicit identity for a folder row: a path can appear in several sidebar trees.

use eframe::egui::Id;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarFolderTarget {
    pub path: PathBuf,
    pub node_id: Id,
    pub name: String,
}

impl SidebarFolderTarget {
    pub fn new(tree_id: Id, path: &Path, name: &str) -> Self {
        Self {
            path: path.to_path_buf(),
            node_id: tree_id.with(path),
            name: name.to_owned(),
        }
    }

    pub fn editor_id(&self) -> Id {
        self.node_id.with("rename")
    }
}

#[derive(Clone, Debug)]
pub struct SidebarFolderRename {
    pub target: SidebarFolderTarget,
    pub text: String,
    pub request_focus: bool,
}

impl SidebarFolderRename {
    pub fn new(target: SidebarFolderTarget) -> Self {
        Self {
            text: target.name.clone(),
            target,
            request_focus: true,
        }
    }

    pub fn matches(&self, node: &SidebarFolderTarget) -> bool {
        self.target.node_id == node.node_id && self.target.path == node.path
    }
}
