use super::{catalog::Command, disk_analysis, file_actions};
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
    pub can_preview_selected: bool,
    pub can_copy: bool,
    pub can_cut: bool,
    pub has_clipboard_files: bool,
    pub can_paste: bool,
    pub can_delete: bool,
    pub disk_analysis_drive: Option<char>,
}

impl CommandContext {
    pub fn from_app(app: &ImageViewerApp) -> Self {
        let has_selection = app.selected_file.is_some() || !app.multi_selection.is_empty();
        let paths = file_actions::selected_paths(app);
        let supported_targets = file_actions::supported_targets(&paths);
        let can_copy = supported_targets && app.can_copy_from_current_location();
        let can_move = supported_targets && app.can_delete_with_shell_paths(&paths);
        let in_archive_namespace = app.current_location_is_archive_namespace();
        let has_clipboard_files = app.clipboard.has_content();
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
            can_preview_selected: app.can_open_selected_preview(),
            can_copy,
            can_cut: can_copy && can_move && !in_archive_namespace,
            has_clipboard_files,
            can_paste: has_clipboard_files && app.can_paste_into_current_location(),
            can_delete: can_move && !in_archive_namespace,
            disk_analysis_drive: disk_analysis::current_drive(app),
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
                && !in_archive_namespace,
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
        Rename | Properties | PreviewSelected | Copy | Cut | Delete if !context.has_selection => {
            Some("command_palette.no_selection")
        }
        PreviewSelected if !context.can_preview_selected => {
            Some("command_palette.preview_unavailable")
        }
        Copy if !context.can_copy => Some("command_palette.unavailable"),
        Cut if !context.can_cut => Some("command_palette.unavailable"),
        Delete if !context.can_delete => Some("command_palette.unavailable"),
        Paste if !context.has_clipboard_files => Some("command_palette.clipboard_empty"),
        Paste if !context.can_paste => Some("command_palette.unavailable"),
        CreateFolder if !context.can_create_folder => Some("command_palette.unavailable"),
        Rename if !context.can_rename => Some("command_palette.unavailable"),
        Properties if !context.can_properties => Some("command_palette.unavailable"),
        DiskAnalysis if context.disk_analysis_drive.is_none() => {
            Some("command_palette.disk_analysis_unavailable")
        }
        _ => None,
    }
}
