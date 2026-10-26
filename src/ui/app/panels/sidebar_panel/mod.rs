use crate::app::ImageViewerApp;
use crate::ui::sidebar::{SidebarAction, SidebarContext};
use eframe::egui;

use super::{sidebar_layout, LEFT_SIDEBAR_MAX, LEFT_SIDEBAR_MIN};

mod independent;
mod quick_access;
mod unified;

pub(super) fn render(app: &mut ImageViewerApp, root_ui: &mut egui::Ui) -> Option<SidebarAction> {
    let started = std::time::Instant::now();
    if !app.show_left_sidebar {
        app.cancel_sidebar_folder_rename();
        return None;
    }
    let target_width = app
        .layout
        .sidebar_left_width
        .clamp(LEFT_SIDEBAR_MIN, LEFT_SIDEBAR_MAX);
    let dark_mode = root_ui.visuals().dark_mode;
    let ctx = root_ui.ctx().clone();
    crate::ui::sidebar_tree::rename::begin_frame(&ctx);
    let can_drop_items_to_recycle_bin = app.can_drop_drag_payload_to_recycle_bin();
    let response = egui::Panel::left("sidebar")
        .exact_size(target_width)
        .resizable(false)
        .frame(egui::Frame::NONE.fill(if dark_mode {
            crate::ui::theme::palette::surface(root_ui.ctx(), egui::Color32::from_rgb(45, 45, 45))
        } else {
            egui::Color32::WHITE
        }))
        .show(root_ui, |ui| {
            // Register before rows so item context menus retain priority.
            let background = ui.interact(
                ui.max_rect(),
                ui.id().with("sidebar_background_menu"),
                egui::Sense::click(),
            );
            let action = if app.explorer_preferences.sidebar_unified_scroll {
                unified::render(app, ui, can_drop_items_to_recycle_bin)
            } else {
                independent::render(app, ui, can_drop_items_to_recycle_bin)
            };
            action.or(crate::ui::sidebar::section_menu::menu(
                &background,
                app.is_item_dragging
                    || app.renaming_state.is_some()
                    || app.sidebar_renaming.is_some()
                    || app.sidebar_folder_rename.is_some(),
            ))
        });

    app.sidebar_rename_focus = false;
    crate::ui::sidebar_tree::rename::finish_frame(&ctx, &mut app.sidebar_folder_rename);
    let action = match response.inner {
        Some(SidebarAction::OpenDriveContextMenu(path)) => {
            app.open_sidebar_context_menu(&ctx, path);
            None
        }
        Some(SidebarAction::OpenTreeContextMenu(target)) => {
            app.open_sidebar_folder_context_menu(&ctx, target);
            None
        }
        other => other,
    };
    let elapsed = started.elapsed().as_millis();
    if elapsed > 50 {
        log::warn!(
            "[PERF-SIDEBAR-PANEL] show={}ms (egui panel + scroll + content)",
            elapsed
        );
    }
    action
}

fn context(app: &mut ImageViewerApp, can_drop_items_to_recycle_bin: bool) -> SidebarContext<'_> {
    let is_folder_dragging = app.is_item_dragging && app.drag_payload_is_single_directory;
    SidebarContext {
        disks: &app.drive_state.disks,
        drive_info_cache: &app.drive_state.drive_info_cache,
        cloud_roots: &app.drive_state.cloud_roots,
        network_locations: &app.drive_state.network_locations,
        current_path: &app.navigation_state.current_path,
        context_selection: crate::ui::sidebar_selection::SidebarContextSelection::from_menu(
            &app.context_menu,
        ),
        is_computer_view: app.navigation_state.is_computer_view,
        is_recycle_bin_view: app.navigation_state.is_recycle_bin_view,
        computer_icon: app.cache_manager.computer_icon.as_ref(),
        is_renaming: app.renaming_state.is_some()
            || app.sidebar_renaming.is_some()
            || app.sidebar_folder_rename.is_some(),
        icon_loader: &mut app.item_icon_loader,
        pinned_folders: &app.pinned_folders,
        is_item_dragging: app.is_item_dragging,
        can_drop_items_to_recycle_bin,
        is_folder_dragging,
        dragging_path: if is_folder_dragging {
            app.drag_payload_paths
                .first()
                .and_then(|path| path.to_str())
        } else {
            None
        },
        show_recycle_bin: app.show_recycle_bin,
        show_quick_access: app.show_quick_access,
        show_tags: app.show_tags,
        collapse_quick_access: app.collapse_quick_access,
        collapse_cloud_drives: app.collapse_cloud_drives,
        collapse_local_disks: app.collapse_local_disks,
        collapse_network_drives: app.collapse_network_drives,
        collapse_network_locations: app.collapse_network_locations,
        sidebar_renaming: app
            .sidebar_renaming
            .as_ref()
            .map(|(path, text)| (path.as_str(), text.as_str())),
        sidebar_rename_focus: app.sidebar_rename_focus,
        folder_rename: app.sidebar_folder_rename.as_ref(),
        mounted_iso_drives: &app.file_operation_state.mounted_iso_drives,
        tree_state: &app.sidebar_tree,
        tag_definitions: &app.tag_definitions,
        sidebar_tag_ids: &app.sidebar_tag_ids,
        tag_counts: &app.tag_counts,
        active_tag_filter: app.active_tag_filter,
        collapse_tags: app.collapse_tags,
    }
}
