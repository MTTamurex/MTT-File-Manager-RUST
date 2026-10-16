//! Context menus and rename dispatch for explicit sidebar folder targets.

use crate::app::ImageViewerApp;
use crate::application::sidebar_tree::{SidebarFolderRename, SidebarFolderTarget};
use std::path::{Path, PathBuf};

impl ImageViewerApp {
    fn sidebar_folder_path_can_be_renamed(&self, path: &Path) -> bool {
        !crate::domain::network_path::is_network_location_root(path)
            && !crate::infrastructure::windows::is_drive_root_path(path)
            && !self.path_is_same_or_ancestor_of_open_panel(path)
    }

    pub(crate) fn can_rename_sidebar_folder(&self, path: &Path) -> bool {
        self.context_menu
            .sidebar_folder_target
            .as_ref()
            .is_some_and(|target| target.path == path)
            && self.sidebar_folder_path_can_be_renamed(path)
    }

    pub(crate) fn begin_sidebar_folder_rename(&mut self, path: &Path) {
        if !self.can_rename_sidebar_folder(path) {
            return;
        }
        if let Some(target) = self.context_menu.sidebar_folder_target.clone() {
            self.sidebar_folder_rename = Some(SidebarFolderRename::new(target));
        }
    }

    pub(crate) fn commit_sidebar_folder_rename(
        &mut self,
        node_id: eframe::egui::Id,
        new_name: String,
    ) {
        if self
            .sidebar_folder_rename
            .as_ref()
            .is_none_or(|editing| editing.target.node_id != node_id)
        {
            return;
        }
        let editing = self.sidebar_folder_rename.take().unwrap();
        self.ui_ctx
            .memory_mut(|memory| memory.surrender_focus(editing.target.editor_id()));
        if self.sidebar_folder_path_can_be_renamed(&editing.target.path) {
            // The editor remains a tree operation after its menu has closed.
            let origin = self.context_menu.origin;
            self.context_menu.origin =
                crate::application::context_menu::ContextMenuOrigin::SidebarTree;
            self.rename_path_with_shell(editing.target.path, new_name);
            self.context_menu.origin = origin;
        }
    }

    pub(crate) fn cancel_sidebar_folder_rename(&mut self) {
        crate::ui::sidebar_tree::rename::cancel(&self.ui_ctx, &mut self.sidebar_folder_rename);
    }

    pub(crate) fn cancel_sidebar_edits(&mut self) {
        self.cancel_sidebar_folder_rename();
        self.sidebar_renaming = None;
        self.sidebar_rename_focus = false;
    }

    pub(crate) fn open_sidebar_context_menu(&mut self, ctx: &eframe::egui::Context, path: String) {
        self.open_sidebar_menu(ctx, PathBuf::from(path), None);
    }

    pub(crate) fn open_sidebar_folder_context_menu(
        &mut self,
        ctx: &eframe::egui::Context,
        target: SidebarFolderTarget,
    ) {
        self.open_sidebar_menu(ctx, target.path.clone(), Some(target));
    }

    fn open_sidebar_menu(
        &mut self,
        ctx: &eframe::egui::Context,
        path: PathBuf,
        folder: Option<SidebarFolderTarget>,
    ) {
        let position = ctx.input(|input| input.pointer.hover_pos().unwrap_or_default());
        self.context_menu
            .open_for_sidebar_tree(position, path.clone());
        self.context_menu.sidebar_folder_target = folder;
        self.capture_context_menu_panel_origin();
        self.populate_context_menu(ctx, &[path], false, None);
    }
}
