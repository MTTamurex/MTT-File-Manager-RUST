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
            use crate::ui::sidebar::{render_sidebar, SidebarContext};

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
                app.context_menu.target_paths[0]
                    .to_str()
                    .filter(|path| {
                        crate::infrastructure::windows::is_drive_root_path(
                            std::path::Path::new(path),
                        )
                    })
            } else {
                None
            };

            let sidebar_renaming_ref = app.sidebar_renaming.as_ref()
                .map(|(p, t)| (p.as_str(), t.as_str()));

            let mut sidebar_ctx = SidebarContext {
                disks: &app.drive_state.disks,
                current_path: &app.navigation_state.current_path,
                highlighted_drive_path,
                is_computer_view,
                is_recycle_bin_view: app.navigation_state.is_recycle_bin_view,
                computer_icon: app.cache_manager.computer_icon.as_ref(),
                is_renaming: app.renaming_state.is_some() || app.sidebar_renaming.is_some(),
                icon_loader: &mut app.item_icon_loader,
                onedrive_path: app.onedrive_path.as_deref(),
                onedrive_icon: app.onedrive_icon.as_ref(),
                pinned_folders: &app.pinned_folders,
                is_item_dragging: app.is_item_dragging,
                is_folder_dragging,
                dragging_path,
                sidebar_renaming: sidebar_renaming_ref,
                sidebar_rename_focus: app.sidebar_rename_focus,
            };

            egui::ScrollArea::vertical()
                .id_salt("sidebar_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| render_sidebar(ui, &mut sidebar_ctx))
                .inner
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
        SidebarAction::CommitDriveRename { drive_path, new_label } => {
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
                if app.file_operation_state.file_op_sender.send(
                    crate::workers::file_operation_worker::FileOperationRequest::rename(
                        path,
                        new_label,
                        app.native_hwnd.unwrap_or_default(),
                    ),
                ).is_err() {
                    app.file_operation_state.file_ops_in_progress =
                        app.file_operation_state.file_ops_in_progress.saturating_sub(1);
                    log::warn!("[FileOps] worker channel closed on sidebar drive rename");
                }
            }
        }
        SidebarAction::CancelDriveRename => {
            app.sidebar_renaming = None;
            app.sidebar_rename_focus = false;
        }
    }
}

fn render_resize_handles(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let screen = ctx.screen_rect();
    let tab_bar_height = 35.0; // Approximate tab bar height

    // Left sidebar resize handle (right edge of left sidebar)
    let left_width = app
        .layout
        .sidebar_left_width
        .clamp(LEFT_SIDEBAR_MIN, LEFT_SIDEBAR_MAX);
    let left_handle_rect = egui::Rect::from_min_size(
        egui::pos2(left_width - RESIZE_HANDLE_WIDTH / 2.0, tab_bar_height),
        egui::vec2(RESIZE_HANDLE_WIDTH, screen.height() - tab_bar_height),
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

    // Right sidebar resize handle (left edge of right sidebar) - only if panel is visible
    if app.show_preview_panel {
        let right_width = app
            .layout
            .sidebar_right_width
            .clamp(RIGHT_SIDEBAR_MIN, RIGHT_SIDEBAR_MAX);
        let right_handle_x = screen.width() - right_width - RESIZE_HANDLE_WIDTH / 2.0;
        let right_handle_rect = egui::Rect::from_min_size(
            egui::pos2(right_handle_x, tab_bar_height),
            egui::vec2(RESIZE_HANDLE_WIDTH, screen.height() - tab_bar_height),
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
