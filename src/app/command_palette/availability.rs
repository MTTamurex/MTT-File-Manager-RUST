use super::catalog::Command;
use crate::app::ImageViewerApp;

/// A metadata-only snapshot; the renderer never performs operations on files.
#[derive(Default)]
pub(crate) struct CommandContext {
    pub can_add_tab: bool,
    pub tab_pinned: bool,
    pub has_closed_tabs: bool,
    pub can_back: bool,
    pub can_forward: bool,
    pub can_up: bool,
    pub view_locked: bool,
    pub has_items: bool,
    pub has_selection: bool,
    pub can_create_folder: bool,
    pub can_rename: bool,
    pub can_properties: bool,
}

impl CommandContext {
    pub fn from_app(app: &ImageViewerApp) -> Self {
        let has_selection = app.selected_file.is_some() || !app.multi_selection.is_empty();
        Self {
            can_add_tab: app.tab_manager.can_add_tab(),
            tab_pinned: app.tab_manager.active().pinned,
            has_closed_tabs: app.tab_manager.has_closed_tabs(),
            can_back: app.can_go_back(),
            can_forward: app.can_go_forward(),
            can_up: app.can_go_up(),
            view_locked: app.current_folder_locked,
            has_items: !app.items.is_empty(),
            has_selection,
            can_create_folder: app.can_create_folder_in_current_location(),
            can_rename: if app.multi_selection.len() > 1 {
                app.can_batch_rename()
            } else {
                app.selected_file.as_ref().is_some_and(|item| {
                    item.drive_info.is_none() && app.can_rename_path(&item.path)
                })
            },
            can_properties: has_selection
                && app.native_hwnd.is_some()
                && !app.navigation_state.is_recycle_bin_view
                && !app.current_location_is_archive_namespace(),
        }
    }
}

pub(crate) fn disabled_reason(command: Command, context: &CommandContext) -> Option<&'static str> {
    use Command::*;
    match command {
        NewTab | ReopenTab if !context.can_add_tab => Some("command_palette.tab_limit"),
        CloseTab if context.tab_pinned => Some("command_palette.pinned_tab"),
        ReopenTab if !context.has_closed_tabs => Some("command_palette.no_closed_tabs"),
        Back if !context.can_back => Some("command_palette.no_history"),
        Forward if !context.can_forward => Some("command_palette.no_history"),
        Up if !context.can_up => Some("command_palette.no_parent"),
        Grid | Details | Columns | Miller | NextView | PreviousView if context.view_locked => {
            Some("command_palette.view_locked")
        }
        SelectAll | InvertSelection if !context.has_items => Some("command_palette.no_items"),
        ClearSelection if !context.has_selection => Some("command_palette.no_selection"),
        Rename | Properties if !context.has_selection => Some("command_palette.no_selection"),
        CreateFolder if !context.can_create_folder => Some("command_palette.unavailable"),
        Rename if !context.can_rename => Some("command_palette.unavailable"),
        Properties if !context.can_properties => Some("command_palette.unavailable"),
        _ => None,
    }
}
