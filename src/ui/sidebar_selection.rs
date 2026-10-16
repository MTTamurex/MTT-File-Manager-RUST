//! Temporary sidebar selection for an open context menu, without navigation.

use crate::application::context_menu::{ContextMenuOrigin, ContextMenuState};
use eframe::egui::Id;
use std::path::Path;

#[derive(Clone, Copy)]
pub struct SidebarContextSelection<'a> {
    path: &'a Path,
    node_id: Option<Id>,
}

impl<'a> SidebarContextSelection<'a> {
    pub fn from_menu(menu: &'a ContextMenuState) -> Option<Self> {
        if !menu.is_open
            || menu.origin != ContextMenuOrigin::SidebarTree
            || menu.target_paths.len() != 1
        {
            return None;
        }
        Some(Self {
            path: &menu.target_paths[0],
            node_id: menu
                .sidebar_folder_target
                .as_ref()
                .map(|target| target.node_id),
        })
    }

    fn matches(self, path: &Path, node_id: Option<Id>) -> bool {
        self.node_id == node_id
            && self
                .path
                .to_string_lossy()
                .eq_ignore_ascii_case(&path.to_string_lossy())
    }
}

pub(crate) fn row_is_selected(
    context: Option<SidebarContextSelection<'_>>,
    path: &Path,
    node_id: Option<Id>,
    navigation_selected: bool,
) -> bool {
    context.map_or(navigation_selected, |selection| {
        selection.matches(path, node_id)
    })
}
