use crate::app::ImageViewerApp;
use crate::domain::file_entry::{FileEntry, SyncStatus, ViewMode};
use crate::domain::special_paths::{COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};
use crate::infrastructure::windows as windows_infra;
use crate::ui::sidebar::SidebarAction;
use eframe::egui;
use std::cell::RefCell;
use std::path::PathBuf;

// M-12: Per-frame cache for "current folder" FileEntry (when no file is selected).
// Keyed by (current_path, modified_hint); invalidated on navigation or folder update.
thread_local! {
    static FOLDER_ENTRY_CACHE: RefCell<Option<(String, u64, FileEntry)>> = RefCell::new(None);
}

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
    render_preview_panel_layout(app, ctx, _frame);
    let preview_ms = t_preview.elapsed().as_millis();

    // 4. Central Panel
    let t_central = std::time::Instant::now();
    render_central_panel_layout(app, ctx);
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
            let is_folder_dragging = app.is_item_dragging
                && app.drag_payload_paths.iter().all(|p| p.is_dir())
                && app.drag_payload_paths.len() == 1;
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

            let mut sidebar_ctx = SidebarContext {
                disks: &app.drive_state.disks,
                current_path: &app.navigation_state.current_path,
                highlighted_drive_path,
                is_computer_view,
                is_recycle_bin_view: app.navigation_state.is_recycle_bin_view,
                computer_icon: app.cache_manager.computer_icon.as_ref(),
                is_renaming: app.renaming_state.is_some(),
                icon_loader: &mut app.item_icon_loader,
                onedrive_path: app.onedrive_path.as_deref(),
                onedrive_icon: app.onedrive_icon.as_ref(),
                pinned_folders: &app.pinned_folders,
                is_item_dragging: app.is_item_dragging,
                is_folder_dragging,
                dragging_path,
            };

            egui::ScrollArea::vertical()
                .id_salt("sidebar_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| render_sidebar(ui, &mut sidebar_ctx))
                .inner
        });

    let sidebar_action = match sidebar_response.inner {
        Some(SidebarAction::OpenDriveContextMenu(path)) => {
            let path_buf = std::path::PathBuf::from(&path);
            let pos = ctx.input(|i| i.pointer.hover_pos().unwrap_or_default());
            let right_bound = sidebar_response.response.rect.right();

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
            // If this path is a pinned folder that no longer exists, auto-unpin + notify
            let is_pinned = app.pinned_folders.iter().any(|pf| pf.path == path);
            if is_pinned && !std::path::Path::new(&path).exists() {
                app.unpin_folder(&path);
                app.notifications.warning(rust_i18n::t!(
                    "panels.folder_removed",
                    name = std::path::Path::new(&path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&path)
                ).to_string());
            } else {
                app.navigate_to(&path);
            }
        }
        SidebarAction::NavigateToComputer => app.navigate_to_computer(),
        SidebarAction::NavigateToRecycleBin => app.navigate_to_recycle_bin(),
        SidebarAction::OpenDriveContextMenu(_) => {}
        SidebarAction::PinFolder(path) => app.pin_folder(&path),
        SidebarAction::UnpinFolder(path) => app.unpin_folder(&path),
        SidebarAction::ReorderPinnedFolder { from, to } => app.reorder_pinned_folder(from, to),
    }
}

fn render_preview_panel_layout(
    app: &mut ImageViewerApp,
    ctx: &egui::Context,
    frame: &eframe::Frame,
) {
    if app.show_preview_panel {
        // M-2: Only call refresh_selected_metadata when the selection actually changed.
        // The function has its own early-return for same-path, but this skips the call
        // entirely (avoiding closure + field reads) on the common no-change frame.
        let needs_metadata_refresh = match (&app.selected_file, &app.last_metadata_path) {
            (Some(f), _) if f.is_dir => false,
            (Some(f), Some(p)) => f.path != *p,
            (Some(_), None) => true, // watcher cleared last_metadata_path to force re-fetch
            (None, _) => false,
        };
        if needs_metadata_refresh {
            app.refresh_selected_metadata();
        }

        // Clamp width to valid range BEFORE using it
        let target_width = app
            .layout
            .sidebar_right_width
            .clamp(RIGHT_SIDEBAR_MIN, RIGHT_SIDEBAR_MAX);

        // Use exact_width + resizable(false) to FORCE the width from app state
        // Resize is handled via manual drag handles rendered separately
        let _right_panel_response = egui::SidePanel::right("preview_panel")
            .exact_width(target_width)
            .resizable(false) // Resize handled manually via drag handles
            .frame(egui::Frame {
                fill: if ctx.style().visuals.dark_mode {
                    egui::Color32::from_rgb(45, 45, 45)
                } else {
                    egui::Color32::WHITE
                },
                inner_margin: egui::Margin {
                    left: 12,
                    right: 12,
                    top: 8,
                    bottom: 8,
                },
                ..Default::default()
            })
            .show(ctx, |ui| {
                use crate::ui::preview_panel::{render_preview_panel, PreviewPanelAction};

                egui::ScrollArea::vertical()
                    .id_salt("preview_scroll")
                    .show(ui, |ui| {
                        ui.set_max_width(ui.available_width());

                        let effective_file = calculate_effective_file(app);

                        if let Some(file) = effective_file {
                            let tab_id = app.tab_manager.active().id;
                            let selected_metadata =
                                app.selected_metadata.as_ref().and_then(|(p, meta)| {
                                    if p == &file.path {
                                        Some(meta)
                                    } else {
                                        None
                                    }
                                });

                            let folder_size = if file.is_dir {
                                app.folder_size_state.cache.peek(&file.path).copied()
                            } else {
                                None
                            };
                            let is_folder_size_loading =
                                app.folder_size_state.loading.contains(&file.path);

                            let is_owner = app.media_preview_owner_tab_id == Some(tab_id);

                            let action = render_preview_panel(
                                ui,
                                &file,
                                app.multi_selection.len(),
                                app.selected_thumbnail.as_ref(),
                                app.selected_gif.as_mut(),
                                app.media_preview.as_mut(), // Always pass mut if it exists, visibility is controlled by HWND
                                selected_metadata,
                                app.cache_manager.texture_cache.peek(&file.path).cloned(),
                                app.cache_manager
                                    .folder_preview_cache
                                    .get(&file.path)
                                    .cloned(),
                                app.cache_manager
                                    .folder_preview_loading
                                    .contains(&file.path),
                                app.metadata_loading.contains(&file.path),
                                folder_size,
                                is_folder_size_loading,
                                app.navigation_state.is_recycle_bin_view,
                                &mut app.item_icon_loader,
                                &mut app.svg_icon_manager,
                                Some(frame),
                                is_owner,
                                app.cache_manager.is_failed(&file.path),
                            );

                            if let Some(act) = action {
                                match act {
                                    PreviewPanelAction::RequestPlay(path) => {
                                        app.request_video_preview_playback(path);
                                    }
                                    PreviewPanelAction::DetachVideo { path, position, volume } => {
                                        // 1. Kill any existing standalone player
                                        app.kill_video_player_process();
                                        // 2. Destroy the in-process media preview (frees MPV/HWND)
                                        app.destroy_media_preview();
                                        // 3. Spawn standalone video player process
                                        if let Some(child) = crate::video_player::open_video_player(path, position, volume) {
                                            app.video_player_process = Some(child);
                                        }
                                    }
                                    PreviewPanelAction::RefreshThumbnail(path) => {
                                        // Check if it's a folder or a file
                                        let is_folder = app
                                            .selected_file
                                            .as_ref()
                                            .map(|f| f.is_dir && !f.is_archive())
                                            .unwrap_or(false);

                                        if is_folder {
                                            // Handle folder preview refresh
                                            log::debug!(
                                                "[REFRESH FOLDER PREVIEW] Starting refresh for: {:?}",
                                                path
                                            );
                                            // Clear folder preview cache
                                            app.cache_manager.folder_preview_cache.pop(&path);
                                            log::debug!("[REFRESH FOLDER PREVIEW] Folder preview cache cleared");
                                            app.cache_manager.finish_folder_preview_loading(&path);
                                            log::debug!("[REFRESH FOLDER PREVIEW] Loading state cleared");
                                            // Re-request folder preview
                                            let _ = app.folder_preview_sender.send(path.clone());
                                            log::debug!(
                                                "[REFRESH FOLDER PREVIEW] Request sent for: {:?}",
                                                path
                                            );
                                            app.notifications.push(
                                                crate::application::AppNotification::info(
                                                    rust_i18n::t!("panels.refreshing_preview").to_string(),
                                                ),
                                            );
                                        } else {
                                            // Handle file thumbnail refresh (existing logic)
                                            log::debug!(
                                                "[REFRESH THUMBNAIL] Starting refresh for: {:?}",
                                                path
                                            );
                                            // Clear all caches to allow retry
                                            // PERF FIX (C-1): Dispatch SQLite cleanup to background worker
                                            // FORCED: user explicitly requested refresh
                                            app.enqueue_disk_cache_invalidations_forced(vec![path.clone()]);
                                            log::debug!("[REFRESH THUMBNAIL] Disk cache cleared");
                                            app.cache_manager.texture_cache.pop(&path);
                                            log::debug!("[REFRESH THUMBNAIL] Texture cache cleared");
                                            app.cache_manager.loading_set.remove(&path);
                                            log::debug!("[REFRESH THUMBNAIL] Loading set cleared");
                                            // CRITICAL: Also clear RAM cache (rgba_data_cache) or
                                            // request_thumbnail_load will return early without re-extracting
                                            app.cache_manager.pop_rgba_data(&path);
                                            log::debug!("[REFRESH THUMBNAIL] RGBA cache cleared");
                                            // Clear failure cache so it will be retried
                                            crate::workers::thumbnail::clear_failure_cache(&path);
                                            log::debug!("[REFRESH THUMBNAIL] Failure cache cleared");
                                            // Force regeneration by passing modified=0 (will trigger new extraction)
                                            app.request_thumbnail_load_with_modified(
                                                path.clone(),
                                                512,
                                                0,
                                            );
                                            log::debug!(
                                                "[REFRESH THUMBNAIL] Request sent to worker for: {:?}",
                                                path
                                            );
                                            app.notifications.push(
                                                crate::application::AppNotification::info(
                                                    rust_i18n::t!("panels.refreshing_thumbnail").to_string(),
                                                ),
                                            );
                                        }
                                    }
                                    PreviewPanelAction::LoadFolderPreview(path) => {
                                        app.request_folder_preview_load(path);
                                    }
                                    PreviewPanelAction::CalculateFolderSize(path) => {
                                        // Cancel any in-progress calculation before starting new one
                                        app.folder_size_state.cancel
                                            .store(true, std::sync::atomic::Ordering::Release);
                                        app.folder_size_state.loading.insert(path.clone());
                                        let _ = app.folder_size_state.req_sender.send(path);
                                    }
                                    PreviewPanelAction::VolumeChanged(vol) => {
                                        app.session_volume = vol;
                                    }
                                }
                            }
                        } else {
                            ui.vertical_centered(|ui| {
                                ui.add_space(100.0);
                                ui.label(rust_i18n::t!("panels.no_selection"));
                                ui.label(rust_i18n::t!("panels.select_hint"));
                            });
                        }
                    });
            });
    }
}

/// Render manual resize handles for sidebars.
/// These are thin vertical areas at the edge of each sidebar that respond to drag.
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

fn calculate_effective_file(app: &ImageViewerApp) -> Option<FileEntry> {
    // PERF FIX (M-1): Return a reference-based approach — clone only when the
    // selected file actually changed since last frame. For the common case
    // (same file selected across frames), we reuse the cached entry.
    if let Some(ref file) = app.selected_file {
        // PERFORMANCE FIX: NEVER call path.exists() in render loop!
        // On HDD with video playing, this causes I/O spikes every frame.
        // File existence is validated on:
        // 1. Selection (when user clicks)
        // 2. File system watcher events (auto-refresh)
        // Trust the cached state - it's updated by the file watcher.
        Some(file.clone())
    } else if app.navigation_state.is_recycle_bin_view {
        Some(FileEntry {
            path: PathBuf::from(RECYCLE_BIN_VIEW_ID),
            name: RECYCLE_BIN_VIEW_ID.to_string(),
            is_dir: true,
            size: 0,
            modified: 0,
            folder_cover: None,
            drive_info: None,
            sync_status: SyncStatus::None,
            is_hidden: false,
            deletion_date: None,
            recycle_original_path: None,
        })
    } else if app.navigation_state.is_computer_view {
        // "Este Computador" - show drive count info
        Some(FileEntry {
            path: PathBuf::from(COMPUTER_VIEW_ID),
            name: COMPUTER_VIEW_ID.to_string(),
            is_dir: true,
            size: app.drive_state.disks.len() as u64, // Store drive count in size field
            modified: 0,
            folder_cover: None,
            drive_info: None,
            sync_status: SyncStatus::None,
            is_hidden: false,
            deletion_date: None,
            recycle_original_path: None,
        })
    } else {
        // M-12: Check cache before rebuilding the folder entry every frame.
        let mod_hint = app
            .current_folder_modified_hint
            .as_ref()
            .and_then(|(hp, m)| {
                if hp.to_string_lossy() == app.navigation_state.current_path.as_str() && *m > 0 {
                    Some(*m)
                } else {
                    None
                }
            })
            .unwrap_or(0u64);
        let cache_hit = FOLDER_ENTRY_CACHE.with(|c| {
            c.borrow().as_ref().and_then(|(cp, cm, e)| {
                if cp == &app.navigation_state.current_path && *cm == mod_hint {
                    Some(e.clone())
                } else {
                    None
                }
            })
        });
        if let Some(entry) = cache_hit {
            return Some(entry);
        }
        let path = std::path::PathBuf::from(&app.navigation_state.current_path);
        let current_folder_modified = mod_hint; // reuse already-computed value
        // CRITICAL FIX: Do NOT use FileEntry::from_path() here!
        // from_path() calls std::fs::metadata() which uses CreateFileW internally.
        // On OneDrive, this can block the UI thread for 30-60s on cloud-only files.
        // This function runs EVERY FRAME — even a 1ms delay compounds.
        // Build the entry with defaults instead; size/modified are not needed for
        // the preview panel display of the current directory.
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let mut entry = FileEntry {
            path: path.clone(),
            name,
            is_dir: true,
            size: 0,
            modified: current_folder_modified,
            folder_cover: None,
            drive_info: None,
            sync_status: SyncStatus::None,
            is_hidden: false,
            deletion_date: None,
            recycle_original_path: None,
        };
        if path.to_string_lossy().len() <= 3 && path.to_string_lossy().contains(':') {
            // PERFORMANCE FIX: Use cached drive_info from items instead of calling
            // get_volume_info() which blocks on network drives EVERY FRAME.
            let label = app
                .drive_state
                .disks
                .iter()
                .find(|(p, _)| p.starts_with(&app.navigation_state.current_path) || app.navigation_state.current_path.starts_with(p))
                .map(|(_, l)| l.clone())
                .unwrap_or_else(|| app.navigation_state.current_path.clone());
            entry.name = label;

            // Try to find cached drive_info from computer view items
            let cached_info = app
                .all_items
                .iter()
                .find(|item| {
                    let item_str = item.path.to_string_lossy();
                    item_str.starts_with(&app.navigation_state.current_path)
                        || app.navigation_state.current_path.starts_with(item_str.as_ref())
                })
                .and_then(|item| item.drive_info.clone())
                // Fallback: persistent drive_info_cache survives navigation away from computer view
                .or_else(|| app.drive_state.drive_info_cache.get(&app.navigation_state.current_path).cloned());

            if let Some(info) = cached_info {
                entry.drive_info = Some(info);
            } else {
                // Fallback NON-BLOCKING: avoid volume probes in render loop.
                // Detailed volume info is filled asynchronously by computer view pipeline.
                let drive_type = windows_infra::detect_drive_type(&app.navigation_state.current_path);
                entry.drive_info = Some(crate::domain::file_entry::DriveInfo {
                    file_system: String::new(),
                    total_space: 0,
                    free_space: 0,
                    drive_type,
                });
            }
        } else {
            entry.name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| app.navigation_state.current_path.clone());
        }
        // M-12: Update cache before returning
        FOLDER_ENTRY_CACHE.with(|c| {
            *c.borrow_mut() = Some((
                app.navigation_state.current_path.clone(),
                mod_hint,
                entry.clone(),
            ));
        });
        Some(entry)
    }
}

fn render_central_panel_layout(app: &mut ImageViewerApp, ctx: &egui::Context) {
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(if ctx.style().visuals.dark_mode {
            egui::Color32::from_rgb(45, 45, 45)
        } else {
            egui::Color32::WHITE
        }))
        .show(ctx, |ui| {
            if app.is_loading_folder && app.items.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(rust_i18n::t!("panels.loading"));
                });

                // During loading, still update drag target so cursor feedback
                // isn't stale from the previous tab's hovered folder.
                if app.is_item_dragging {
                    app.update_item_drag_target_from_hover(None);
                    let (ctrl, shift, primary_released) = ui.input(|i| {
                        (
                            i.modifiers.ctrl,
                            i.modifiers.shift,
                            i.pointer.primary_released(),
                        )
                    });
                    if primary_released {
                        app.complete_item_drag(ctrl, shift);
                    }
                }
            } else if app.items.is_empty() {
                let response = ui
                    .centered_and_justified(|ui| {
                        ui.label(rust_i18n::t!("panels.empty_folder"));
                    })
                    .response
                    .on_hover_cursor(egui::CursorIcon::Default);

                // During an active drag, update the drop target to the current folder
                // even though there are no items to hover over.
                if app.is_item_dragging {
                    app.update_item_drag_target_from_hover(None);
                    let (ctrl, shift, primary_released) = ui.input(|i| {
                        (
                            i.modifiers.ctrl,
                            i.modifiers.shift,
                            i.pointer.primary_released(),
                        )
                    });
                    if primary_released {
                        app.complete_item_drag(ctrl, shift);
                    }
                }

                // Handle context menu on empty area
                let interact_response = ui
                    .interact(
                        response.rect,
                        ui.id().with("empty_bg"),
                        egui::Sense::click(),
                    )
                    .on_hover_cursor(egui::CursorIcon::Default); // Force cursor on the interaction rect

                if interact_response.secondary_clicked() {
                    app.context_menu.target_paths.clear();

                    // Use current path for shell menu
                    let paths = if app.navigation_state.is_recycle_bin_view {
                        vec![]
                    } else {
                        vec![std::path::PathBuf::from(&app.navigation_state.current_path)]
                    };

                    // Prepare state
                    let pos = ui.input(|i| i.pointer.hover_pos().unwrap_or_default());
                    let right_bound = ui.available_rect_before_wrap().right();

                    // Set state first
                    app.context_menu
                        .open(pos, right_bound, None, paths.clone(), true);

                    // Then populate items
                    app.populate_context_menu(ui.ctx(), &paths, true, None);
                }
            } else {
                let t_view_render = std::time::Instant::now();
                match app.view_mode {
                    ViewMode::Grid => app.render_grid_view(ui),
                    ViewMode::List => app.render_list_view(ui),
                }
                let view_ms = t_view_render.elapsed().as_millis();
                if view_ms > 120 {
                    log::warn!(
                        "[PERF-CENTRAL] Slow list/grid render: {}ms view={:?} items={} all_items={} search_len={} loading={} pending_thumbs={} pending_uploads={} visible_range={:?}",
                        view_ms,
                        app.view_mode,
                        app.items.len(),
                        app.all_items.len(),
                        app.search_query.len(),
                        app.is_loading_folder,
                        app.pending_thumbnails.len(),
                        app.cache_manager.pending_upload_set.len(),
                        app.visible_index_range,
                    );
                }

                if !ui.ctx().wants_keyboard_input()
                    && ui.input(|i| i.key_pressed(egui::Key::F2))
                {
                    if let Some(idx) = app.selected_item {
                        app.begin_rename_item(idx);
                    }
                }

                if app.is_loading_folder {
                    let rect = ui.max_rect();
                    let status_rect = egui::Rect::from_min_size(
                        rect.right_bottom() - egui::vec2(124.0, 22.0),
                        egui::vec2(110.0, 16.0),
                    );
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(status_rect), |ui| {
                        ui.label(
                            egui::RichText::new(rust_i18n::t!("panels.updating").to_string())
                                .size(11.0)
                                .color(egui::Color32::from_gray(130)),
                        );
                    });
                }
            }
        });
}

