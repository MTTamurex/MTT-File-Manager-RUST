use crate::app::ImageViewerApp;
use crate::ui::sidebar::SidebarAction;
use eframe::egui;

mod content;

// Sidebar width constraints
const LEFT_SIDEBAR_MIN: f32 = 150.0;
const LEFT_SIDEBAR_MAX: f32 = 500.0;
const RIGHT_SIDEBAR_MIN: f32 = 250.0;
const RIGHT_SIDEBAR_MAX: f32 = 500.0;
const RESIZE_HANDLE_WIDTH: f32 = 6.0;

pub fn render_panels(app: &mut ImageViewerApp, ctx: &egui::Context, _frame: &mut eframe::Frame) {
    let t_panels_start = std::time::Instant::now();

    // 1. Manual resize handles (rendered FIRST so Foreground Windows appearing later stack ON TOP)
    let t_resize = std::time::Instant::now();
    render_resize_handles(app, ctx);
    let resize_ms = t_resize.elapsed().as_millis();

    // 2. Left Sidebar (forced width from app state)
    let t_sidebar = std::time::Instant::now();
    let sidebar_action = render_sidebar_panel(app, ctx);
    let sidebar_ms = t_sidebar.elapsed().as_millis();

    // Handle sidebar action OUTSIDE the sidebar timing to avoid attributing
    // navigate_to I/O (watch_current_folder, etc.) to sidebar render time.
    if let Some(action) = sidebar_action {
        handle_sidebar_action(app, action);
    }

    // 3. Right Preview Panel (forced width from app state)
    let t_preview = std::time::Instant::now();
    content::render_preview_panel_layout(app, ctx, _frame);
    let preview_ms = t_preview.elapsed().as_millis();

    // 4. Central Panel
    let t_central = std::time::Instant::now();
    content::render_central_panel_layout(app, ctx);
    let central_ms = t_central.elapsed().as_millis();

    // 5. Focus release: When user clicks anywhere outside the video player,
    // release focus back to the main window (MPV no-op, kept for parity)
    #[cfg(target_os = "windows")]
    {
        use crate::ui::components::MediaPreview;

        if ctx.input(|i| i.pointer.any_pressed()) {
            // User clicked somewhere - release player focus
            if let Some(MediaPreview::Video(ref player)) = app.media_preview {
                player.release_focus_auto();
            }
        }
    }

    let total_ms = t_panels_start.elapsed().as_millis();
    if total_ms > 120 {
        let items_len = app.items.len();
        let all_items_len = app.all_items.len();
        let pending_thumbs = app.pending_thumbnails.len();
        let loading_icons = app.loading_icons.len();
        log::warn!(
            "[PERF] Slow render_panels breakdown: resize={}ms sidebar={}ms preview={}ms central={}ms total={}ms | view={:?} items={} all_items={} loading_folder={} pending_thumbs={} loading_icons={}",
            resize_ms,
            sidebar_ms,
            preview_ms,
            central_ms,
            total_ms,
            app.view_mode,
            items_len,
            all_items_len,
            app.is_loading_folder,
            pending_thumbs,
            loading_icons,
        );
    }
}

fn render_sidebar_panel(app: &mut ImageViewerApp, ctx: &egui::Context) -> Option<SidebarAction> {
    let t_sidebar_fn = std::time::Instant::now();
    if !app.show_left_sidebar {
        return None;
    }

    // Clamp width to valid range BEFORE using it
    let target_width = app
        .layout
        .sidebar_left_width
        .clamp(LEFT_SIDEBAR_MIN, LEFT_SIDEBAR_MAX);

    // Use exact_width + resizable(false) to FORCE the width from app state
    // Resize is handled via manual drag handles rendered separately
    let sidebar_response = egui::SidePanel::left("sidebar")
        .exact_width(target_width)
        .resizable(false) // Resize handled manually via drag handles
        .frame(egui::Frame::NONE.fill(if ctx.style().visuals.dark_mode {
            egui::Color32::from_rgb(45, 45, 45)
        } else {
            egui::Color32::WHITE
        }))
        .show(ctx, |ui| {
            use crate::ui::sidebar::{
                render_sidebar_drives, render_sidebar_fixed_top, render_tags_section,
                SidebarContext,
            };

            let is_computer_view = app.navigation_state.is_computer_view;
            let is_folder_dragging = app.is_item_dragging && app.drag_payload_is_single_directory;
            // H-1: borrow directly — no String/Vec/TextureHandle clone per frame
            let dragging_path: Option<&str> = if is_folder_dragging {
                app.drag_payload_paths.first().and_then(|p| p.to_str())
            } else {
                None
            };
            let highlighted_drive_path = if app.context_menu.is_open
                && app.context_menu.target_paths.len() == 1
            {
                app.context_menu.target_paths[0].to_str().filter(|path| {
                    crate::infrastructure::windows::is_drive_root_path(std::path::Path::new(path))
                })
            } else {
                None
            };

            let sidebar_renaming_ref = app
                .sidebar_renaming
                .as_ref()
                .map(|(p, t)| (p.as_str(), t.as_str()));

            // ── Tags section sizing ──
            const TAG_ROW_H: f32 = 26.0;
            const TAG_HEADER_H: f32 = 22.0;
            const TAG_HEADER_GAP_H: f32 = 4.0;
            const TAG_BOTTOM_PADDING_H: f32 = 8.0;
            const TAG_DIVIDER_H: f32 = 8.0 + 9.0 + 8.0; // space + separator + space
            let tag_count = app.tag_definitions.len();
            let tags_collapsed = app.collapse_tags;
            let tags_visible = app.show_tags && tag_count > 0;
            let tags_content_h = if !tags_visible {
                0.0
            } else if tags_collapsed {
                TAG_HEADER_H + TAG_BOTTOM_PADDING_H
            } else {
                TAG_HEADER_H
                    + TAG_HEADER_GAP_H
                    + (tag_count as f32 * TAG_ROW_H)
                    + TAG_BOTTOM_PADDING_H
            };

            // ── Smooth scroll input ──
            const SIDEBAR_SCROLL_SPEED: f32 = 5.0;
            let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
            let sidebar_rect = ui.max_rect();
            let pointer_in_sidebar = ui.input(|i| {
                i.pointer
                    .hover_pos()
                    .map(|p| sidebar_rect.contains(p))
                    .unwrap_or(false)
            });

            // ── Fixed top: This PC + Quick Access (does not scroll) ──
            let top_action = {
                let mut sidebar_ctx = SidebarContext {
                    disks: &app.drive_state.disks,
                    cloud_roots: &app.drive_state.cloud_roots,
                    current_path: &app.navigation_state.current_path,
                    highlighted_drive_path,
                    is_computer_view,
                    is_recycle_bin_view: app.navigation_state.is_recycle_bin_view,
                    computer_icon: app.cache_manager.computer_icon.as_ref(),
                    is_renaming: app.renaming_state.is_some() || app.sidebar_renaming.is_some(),
                    icon_loader: &mut app.item_icon_loader,
                    pinned_folders: &app.pinned_folders,
                    is_item_dragging: app.is_item_dragging,
                    is_folder_dragging,
                    dragging_path,
                    show_recycle_bin: app.show_recycle_bin,
                    show_tags: app.show_tags,
                    collapse_quick_access: app.collapse_quick_access,
                    collapse_cloud_drives: app.collapse_cloud_drives,
                    collapse_local_disks: app.collapse_local_disks,
                    collapse_network_drives: app.collapse_network_drives,
                    sidebar_renaming: sidebar_renaming_ref,
                    sidebar_rename_focus: app.sidebar_rename_focus,
                    mounted_iso_drives: &app.file_operation_state.mounted_iso_drives,
                    tree_state: &app.sidebar_tree,
                    tag_definitions: &app.tag_definitions,
                    tag_counts: &app.tag_counts,
                    active_tag_filter: app.active_tag_filter,
                    collapse_tags: app.collapse_tags,
                };
                render_sidebar_fixed_top(ui, &mut sidebar_ctx)
            };

            // ── Compute actual tags height after fixed top is rendered ──
            let avail_after_top = ui.available_height();
            let tags_scroll_h = if tag_count == 0 {
                0.0
            } else {
                tags_content_h.min(avail_after_top * 0.35)
            };
            let tags_block_h = if tags_scroll_h <= 0.0 {
                0.0
            } else {
                TAG_DIVIDER_H + tags_scroll_h
            };
            let drives_avail = (avail_after_top - tags_block_h).max(0.0);
            let tags_top_y = ui.cursor().top() + drives_avail;
            let pointer_over_tags = ui.input(|i| {
                i.pointer
                    .hover_pos()
                    .map(|p| {
                        p.x >= sidebar_rect.min.x
                            && p.x <= sidebar_rect.max.x
                            && p.y >= tags_top_y
                            && p.y <= sidebar_rect.max.y
                    })
                    .unwrap_or(false)
            });

            if scroll_delta != 0.0 && pointer_in_sidebar && !pointer_over_tags {
                app.sidebar_tree.scroll_target_y += -scroll_delta * SIDEBAR_SCROLL_SPEED;
                if app.sidebar_tree.scroll_target_y < 0.0 {
                    app.sidebar_tree.scroll_target_y = 0.0;
                }
            }

            // Animate visual scroll toward target
            let dt = ui.input(|i| i.predicted_dt).min(0.05);
            let t = (dt * 9.0).min(1.0);
            let diff = app.sidebar_tree.scroll_target_y - app.sidebar_tree.scroll_visual_y;
            if diff.abs() > 1.0 {
                app.sidebar_tree.scroll_visual_y += diff * t;
            } else {
                app.sidebar_tree.scroll_visual_y = app.sidebar_tree.scroll_target_y;
            }

            if (app.sidebar_tree.scroll_visual_y - app.sidebar_tree.scroll_target_y).abs() > 0.5 {
                ui.ctx().request_repaint();
            }

            let scroll_offset = app.sidebar_tree.scroll_visual_y;

            let mut sidebar_ctx = SidebarContext {
                disks: &app.drive_state.disks,
                cloud_roots: &app.drive_state.cloud_roots,
                current_path: &app.navigation_state.current_path,
                highlighted_drive_path,
                is_computer_view,
                is_recycle_bin_view: app.navigation_state.is_recycle_bin_view,
                computer_icon: app.cache_manager.computer_icon.as_ref(),
                is_renaming: app.renaming_state.is_some() || app.sidebar_renaming.is_some(),
                icon_loader: &mut app.item_icon_loader,
                pinned_folders: &app.pinned_folders,
                is_item_dragging: app.is_item_dragging,
                is_folder_dragging,
                dragging_path,
                show_recycle_bin: app.show_recycle_bin,
                show_tags: app.show_tags,
                collapse_quick_access: app.collapse_quick_access,
                collapse_cloud_drives: app.collapse_cloud_drives,
                collapse_local_disks: app.collapse_local_disks,
                collapse_network_drives: app.collapse_network_drives,
                sidebar_renaming: sidebar_renaming_ref,
                sidebar_rename_focus: app.sidebar_rename_focus,
                mounted_iso_drives: &app.file_operation_state.mounted_iso_drives,
                tree_state: &app.sidebar_tree,
                tag_definitions: &app.tag_definitions,
                tag_counts: &app.tag_counts,
                active_tag_filter: app.active_tag_filter,
                collapse_tags: app.collapse_tags,
            };

            // ── Scrollable middle: Cloud drives + Local disks + Network + folder trees ──
            let output = egui::ScrollArea::both()
                .id_salt("sidebar_scroll")
                .auto_shrink([false, false])
                .max_height(drives_avail)
                .min_scrolled_height(0.0)
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .vertical_scroll_offset(scroll_offset)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    render_sidebar_drives(ui, &mut sidebar_ctx)
                });

            // ── Fixed bottom: Tags section (own scrollbar for many tags) ──
            let tags_action = if tags_scroll_h > 0.0 {
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);

                egui::ScrollArea::vertical()
                    .id_salt("sidebar_tags_scroll")
                    .auto_shrink([false, false])
                    .max_height(tags_scroll_h)
                    .min_scrolled_height(0.0)
                    .scroll_bar_visibility(
                        egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                    )
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        let mut act = None;
                        render_tags_section(ui, &mut sidebar_ctx, &mut act);
                        act
                    })
                    .inner
            } else {
                None
            };

            // sidebar_ctx no longer used — borrows of app released
            let _ = sidebar_ctx;

            // Clamp target to actual content bounds after rendering
            let max_scroll = (output.content_size.y - output.inner_rect.height()).max(0.0);
            if app.sidebar_tree.scroll_target_y > max_scroll {
                app.sidebar_tree.scroll_target_y = max_scroll;
            }
            if app.sidebar_tree.scroll_visual_y > max_scroll {
                app.sidebar_tree.scroll_visual_y = max_scroll;
            }

            // Egui also processes scroll delta internally (1x speed) on top of
            // our forced offset.  Undo its contribution so only our 5x-smoothed
            // version applies, but allow genuine external changes (scrollbar drag,
            // scroll_to_me) to pass through.
            let actual_offset = output.state.offset.y;
            let egui_native_drift = actual_offset - scroll_offset;
            if scroll_delta != 0.0
                && pointer_in_sidebar
                && !pointer_over_tags
                && egui_native_drift.abs() > 0.5
            {
                // This drift is from egui double-processing the same wheel event — ignore it.
            } else if egui_native_drift.abs() > 2.0 {
                // Genuine external scroll (scrollbar drag, etc.) — sync to it.
                app.sidebar_tree.scroll_target_y = actual_offset;
                app.sidebar_tree.scroll_visual_y = actual_offset;
            }

            // Prefer fixed top action; fall back to drives, then tags
            top_action.or(output.inner).or(tags_action)
        });

    // Consume focus flag after rendering so it only fires once
    app.sidebar_rename_focus = false;

    let sidebar_action = match sidebar_response.inner {
        Some(SidebarAction::OpenDriveContextMenu(path)) => {
            let path_buf = std::path::PathBuf::from(&path);
            let pos = ctx.input(|i| i.pointer.hover_pos().unwrap_or_default());
            // Use screen width (not sidebar edge) so submenus open to the right
            // into the available central area, not flip left off-screen.
            let right_bound = ctx.screen_rect().right() - app.layout.sidebar_right_width;

            app.context_menu
                .open(pos, right_bound, None, vec![path_buf.clone()], false);
            app.populate_context_menu(ctx, &[path_buf], false, None);
            None
        }
        other => other,
    };

    let show_ms = t_sidebar_fn.elapsed().as_millis();
    if show_ms > 50 {
        log::warn!(
            "[PERF-SIDEBAR-PANEL] show={}ms (egui panel + scroll + content)",
            show_ms,
        );
    }

    sidebar_action
}

/// Handle sidebar actions separately from sidebar rendering to avoid
/// attributing navigate_to I/O to sidebar render time.
fn handle_sidebar_action(app: &mut ImageViewerApp, action: SidebarAction) {
    match action {
        SidebarAction::NavigateTo(path) => {
            // Avoid synchronous existence probes on the UI thread. Deleted pinned folders
            // are pruned by the existing async cleanup path.
            app.navigate_to(&path);
        }
        SidebarAction::NavigateToComputer => app.navigate_to_computer(),
        SidebarAction::NavigateToRecycleBin => app.navigate_to_recycle_bin(),
        SidebarAction::OpenDriveContextMenu(_) => {}
        SidebarAction::PinFolder(path) => app.pin_folder(&path),
        SidebarAction::UnpinFolder(path) => app.unpin_folder(&path),
        SidebarAction::ReorderPinnedFolder { from, to } => app.reorder_pinned_folder(from, to),
        SidebarAction::CommitDriveRename {
            drive_path,
            new_label,
        } => {
            if drive_path.is_empty() {
                // Text update only — update the editable buffer
                if let Some((_, ref mut text)) = app.sidebar_renaming {
                    *text = new_label;
                }
            } else {
                // Actual commit
                let path = std::path::PathBuf::from(&drive_path);
                app.sidebar_renaming = None;
                app.sidebar_rename_focus = false;
                // Dispatch rename to background worker
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
        SidebarAction::TreeToggleExpand(path) => {
            app.sidebar_tree.toggle_expand(&path);
        }
        SidebarAction::ToggleQuickAccess => {
            app.collapse_quick_access = !app.collapse_quick_access;
        }
        SidebarAction::ToggleCloudDrives => {
            app.collapse_cloud_drives = !app.collapse_cloud_drives;
        }
        SidebarAction::ToggleLocalDisks => {
            app.collapse_local_disks = !app.collapse_local_disks;
        }
        SidebarAction::ToggleNetworkDrives => {
            app.collapse_network_drives = !app.collapse_network_drives;
        }
        SidebarAction::FilterByTag(tag_id) => {
            app.set_tag_filter(tag_id);
        }
        SidebarAction::ToggleTags => {
            app.collapse_tags = !app.collapse_tags;
        }
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
    }
}

fn render_resize_handles(app: &mut ImageViewerApp, ctx: &egui::Context) {
    // Skip resize handles when a modal/overlay is active to prevent click-through
    if app.navigation_state.show_settings_window {
        return;
    }

    let screen = ctx.screen_rect();
    // Total height of all top panels: tab bar (36) + toolbar (46) + secondary toolbar (46)
    let top_panels_height = 36.0 + 46.0 + 46.0;

    if app.show_left_sidebar {
        // Left sidebar resize handle (right edge of left sidebar)
        let left_width = app
            .layout
            .sidebar_left_width
            .clamp(LEFT_SIDEBAR_MIN, LEFT_SIDEBAR_MAX);
        let left_handle_rect = egui::Rect::from_min_size(
            egui::pos2(left_width - RESIZE_HANDLE_WIDTH / 2.0, top_panels_height),
            egui::vec2(RESIZE_HANDLE_WIDTH, screen.height() - top_panels_height),
        );

        egui::Area::new(egui::Id::new("left_sidebar_resize"))
            .fixed_pos(left_handle_rect.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let response = ui.allocate_rect(left_handle_rect, egui::Sense::drag());

                // Set cursor on hover/drag
                if response.hovered() || response.dragged() {
                    ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }

                // Update width on drag
                if response.dragged() {
                    let delta = response.drag_delta().x;
                    app.layout.sidebar_left_width = (app.layout.sidebar_left_width + delta)
                        .clamp(LEFT_SIDEBAR_MIN, LEFT_SIDEBAR_MAX);
                }
            });
    }

    // Right sidebar resize handle (left edge of right sidebar) - only if panel is visible
    if app.show_preview_panel {
        let right_width = app
            .layout
            .sidebar_right_width
            .clamp(RIGHT_SIDEBAR_MIN, RIGHT_SIDEBAR_MAX);
        let right_handle_x = screen.width() - right_width - RESIZE_HANDLE_WIDTH / 2.0;
        let right_handle_rect = egui::Rect::from_min_size(
            egui::pos2(right_handle_x, top_panels_height),
            egui::vec2(RESIZE_HANDLE_WIDTH, screen.height() - top_panels_height),
        );

        egui::Area::new(egui::Id::new("right_sidebar_resize"))
            .fixed_pos(right_handle_rect.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let response = ui.allocate_rect(right_handle_rect, egui::Sense::drag());

                // Set cursor on hover/drag
                if response.hovered() || response.dragged() {
                    ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }

                // Update width on drag (note: dragging LEFT increases right panel width)
                if response.dragged() {
                    let delta = -response.drag_delta().x; // Inverted for right panel
                    app.layout.sidebar_right_width = (app.layout.sidebar_right_width + delta)
                        .clamp(RIGHT_SIDEBAR_MIN, RIGHT_SIDEBAR_MAX);
                }
            });
    }
}
