use crate::application::sidebar_tree::SidebarFolderTarget;
use crate::ui::sidebar::SidebarAction;

pub enum SidebarTreeAction {
    OpenInNewTab(String),
    NavigateTo(String),
    ToggleExpand(std::path::PathBuf),
    DropItemsTo(String),
    OpenContextMenu(SidebarFolderTarget),
    RenameChanged(String),
    CommitRename {
        node_id: eframe::egui::Id,
        new_name: String,
    },
    CancelRename,
}

impl From<SidebarTreeAction> for SidebarAction {
    fn from(action: SidebarTreeAction) -> Self {
        match action {
            SidebarTreeAction::OpenInNewTab(path) => Self::OpenInNewTab(path),
            SidebarTreeAction::NavigateTo(path) => Self::NavigateTo(path),
            SidebarTreeAction::ToggleExpand(path) => Self::TreeToggleExpand(path),
            SidebarTreeAction::DropItemsTo(path) => Self::DropItemsTo(path),
            SidebarTreeAction::OpenContextMenu(target) => Self::OpenTreeContextMenu(target),
            SidebarTreeAction::RenameChanged(name) => Self::TreeRenameChanged(name),
            SidebarTreeAction::CommitRename { node_id, new_name } => {
                Self::CommitTreeRename { node_id, new_name }
            }
            SidebarTreeAction::CancelRename => Self::CancelTreeRename,
        }
    }
}
