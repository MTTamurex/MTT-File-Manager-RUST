use super::sidebar_layout;
use crate::app::ImageViewerApp;
use crate::ui::sidebar::SidebarAction;

pub(super) fn handle(app: &mut ImageViewerApp, action: SidebarAction) {
    let sections_before = matches!(
        &action,
        SidebarAction::ToggleCloudDrives
            | SidebarAction::ToggleLocalDisks
            | SidebarAction::ToggleNetworkDrives
            | SidebarAction::ToggleNetworkLocations
    )
    .then(|| sidebar_drives_sections(app));
    match action {
        SidebarAction::SetAllSectionsCollapsed(collapsed) => {
            crate::ui::sidebar::section_menu::apply(app, collapsed);
        }
        SidebarAction::OpenInNewTab(path) => {
            app.open_location_in_new_tab(app.dual_panel_active, &path);
        }
        SidebarAction::NavigateTo(path) => app.navigate_to(&path),
        SidebarAction::NavigateToComputer => app.navigate_to_computer(),
        SidebarAction::NavigateToRecycleBin => app.navigate_to_recycle_bin(),
        SidebarAction::OpenDriveContextMenu(_) => {}
        SidebarAction::OpenComputerContextMenu => {}
        SidebarAction::OpenTreeContextMenu(_) => {}
        SidebarAction::TreeRenameChanged(name) => {
            if let Some(editing) = app.sidebar_folder_rename.as_mut() {
                editing.text = name;
            }
        }
        SidebarAction::CommitTreeRename { node_id, new_name } => {
            app.commit_sidebar_folder_rename(node_id, new_name);
        }
        SidebarAction::CancelTreeRename => app.cancel_sidebar_folder_rename(),
        SidebarAction::PinFolder(path) => app.pin_folder(&path),
        SidebarAction::UnpinFolder(path) => app.unpin_folder(&path),
        SidebarAction::ReorderPinnedFolder { from, to } => app.reorder_pinned_folder(from, to),
        SidebarAction::ReorderTag {
            tag_id,
            before_tag_id,
        } => app.reorder_sidebar_tag(tag_id, before_tag_id),
        SidebarAction::CommitDriveRename {
            drive_path,
            new_label,
        } => {
            if drive_path.is_empty() {
                if let Some((_, ref mut text)) = app.sidebar_renaming {
                    *text = new_label;
                }
            } else {
                let path = std::path::PathBuf::from(&drive_path);
                app.sidebar_renaming = None;
                app.sidebar_rename_focus = false;
                app.file_operation_state.file_ops_in_progress += 1;
                if app
                    .file_operation_state
                    .file_op_sender
                    .send(
                        crate::workers::file_operation_worker::FileOperationRequest::rename(
                            path,
                            new_label,
                            app.native_hwnd.unwrap_or_default(),
                        ),
                    )
                    .is_err()
                {
                    app.file_operation_state.file_ops_in_progress = app
                        .file_operation_state
                        .file_ops_in_progress
                        .saturating_sub(1);
                    log::warn!("[FileOps] worker channel closed on sidebar drive rename");
                }
            }
        }
        SidebarAction::CancelDriveRename => {
            app.sidebar_renaming = None;
            app.sidebar_rename_focus = false;
        }
        SidebarAction::EjectDrive(path) => app.eject_mounted_iso_drive(&path),
        SidebarAction::TreeToggleExpand(path) => app.sidebar_tree.toggle_expand(&path),
        SidebarAction::ToggleQuickAccess => app.collapse_quick_access = !app.collapse_quick_access,
        SidebarAction::ToggleCloudDrives => app.collapse_cloud_drives = !app.collapse_cloud_drives,
        SidebarAction::ToggleLocalDisks => app.collapse_local_disks = !app.collapse_local_disks,
        SidebarAction::ToggleNetworkDrives => {
            app.collapse_network_drives = !app.collapse_network_drives
        }
        SidebarAction::ToggleNetworkLocations => {
            app.collapse_network_locations = !app.collapse_network_locations
        }
        SidebarAction::FilterByTag(tag_id) => app.set_tag_filter(tag_id),
        SidebarAction::ToggleTags => app.collapse_tags = !app.collapse_tags,
        SidebarAction::DropItemsTo(path) => {
            if app.is_item_dragging && !app.file_panel_input_blocked_by_drag_move_confirmation() {
                let target = std::path::PathBuf::from(&path);
                if app.is_valid_drop_target(&target) {
                    app.drag_target_folder = Some(target);
                    let (ctrl, shift) = app.ui_ctx.input(|inp| {
                        (
                            inp.modifiers.ctrl || inp.modifiers.command,
                            inp.modifiers.shift,
                        )
                    });
                    app.complete_item_drag(ctrl, shift);
                } else {
                    app.cancel_item_drag();
                }
            }
        }
        SidebarAction::DropItemsToRecycleBin => {
            if app.can_drop_drag_payload_to_recycle_bin() {
                app.complete_item_drag_to_recycle_bin();
            } else {
                app.cancel_item_drag();
            }
        }
    }
    if let Some(previous) = sections_before {
        let current = sidebar_drives_sections(app);
        sidebar_layout::restore_drives_height(
            previous,
            current,
            &mut app.layout.sidebar_drives_height,
            &mut app.layout.sidebar_drives_heights,
        );
    }
}

fn sidebar_drives_sections(app: &ImageViewerApp) -> [bool; 4] {
    [
        app.collapse_cloud_drives,
        app.collapse_local_disks,
        app.collapse_network_drives,
        app.collapse_network_locations,
    ]
}
