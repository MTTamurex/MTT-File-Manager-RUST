use eframe::egui;
use std::path::PathBuf;
use crate::domain::file_entry::{FileEntry, SyncStatus, ViewMode};
use crate::infrastructure::windows as windows_infra;
use crate::app::ImageViewerApp;

// Sidebar width constraints
const LEFT_SIDEBAR_MIN: f32 = 150.0;
const LEFT_SIDEBAR_MAX: f32 = 500.0;
const RIGHT_SIDEBAR_MIN: f32 = 250.0;
const RIGHT_SIDEBAR_MAX: f32 = 500.0;
const RESIZE_HANDLE_WIDTH: f32 = 6.0;

pub fn render_panels(app: &mut ImageViewerApp, ctx: &egui::Context, _frame: &mut eframe::Frame) {
    // 1. Manual resize handles (rendered FIRST so Foreground Windows appearing later stack ON TOP)
    render_resize_handles(app, ctx);

    // 2. Left Sidebar (forced width from app state)
    render_sidebar_panel(app, ctx);

    // 3. Right Preview Panel (forced width from app state)
    render_preview_panel_layout(app, ctx, _frame);

    // 4. Central Panel
    render_central_panel_layout(app, ctx);
    
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
}

fn render_sidebar_panel(app: &mut ImageViewerApp, ctx: &egui::Context) {
    // Clamp width to valid range BEFORE using it
    let target_width = app.sidebar_left_width.clamp(LEFT_SIDEBAR_MIN, LEFT_SIDEBAR_MAX);
    
    // Use exact_width + resizable(false) to FORCE the width from app state
    // Resize is handled via manual drag handles rendered separately
    let sidebar_response = egui::SidePanel::left("sidebar")
        .exact_width(target_width)
        .resizable(false)  // Resize handled manually via drag handles
        .show(ctx, |ui| {
            use crate::ui::sidebar::{render_sidebar, SidebarContext};

            let disks = app.disks.clone();
            let current_path = app.current_path.clone();
            let is_computer_view = app.is_computer_view;
            let computer_icon = app.cache_manager.computer_icon.clone();

            let mut sidebar_ctx = SidebarContext {
                disks: &disks,
                current_path: &current_path,
                is_computer_view,
                is_recycle_bin_view: app.is_recycle_bin_view,
                computer_icon: computer_icon.as_ref(),
                is_renaming: app.renaming_state.is_some(),
                icon_loader: &mut app.item_icon_loader,
                onedrive_path: app.onedrive_path.as_deref(),
                onedrive_icon: app.onedrive_icon.as_ref(),
            };

            render_sidebar(ui, &mut sidebar_ctx)
        });

    if let Some(action) = sidebar_response.inner {
        use crate::ui::sidebar::SidebarAction;
        match action {
            SidebarAction::NavigateTo(path) => app.navigate_to(&path),
            SidebarAction::NavigateToComputer => app.navigate_to_computer(),
            SidebarAction::NavigateToRecycleBin => app.navigate_to_recycle_bin(),
        }
    }
}

fn render_preview_panel_layout(app: &mut ImageViewerApp, ctx: &egui::Context, frame: &eframe::Frame) {
    if app.show_preview_panel {
        app.refresh_selected_metadata();

        // Clamp width to valid range BEFORE using it
        let target_width = app.sidebar_right_width.clamp(RIGHT_SIDEBAR_MIN, RIGHT_SIDEBAR_MAX);

        // Use exact_width + resizable(false) to FORCE the width from app state
        // Resize is handled via manual drag handles rendered separately
        let _right_panel_response = egui::SidePanel::right("preview_panel")
            .exact_width(target_width)
            .resizable(false)  // Resize handled manually via drag handles
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
                                    if p == &file.path { Some(meta) } else { None }
                                });

                            let folder_size = if file.is_dir {
                                app.folder_size_cache.get(&file.path).copied()
                            } else {
                                None
                            };
                            let is_folder_size_loading =
                                app.folder_size_loading.contains(&file.path);

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
                                app.cache_manager.folder_preview_cache.get(&file.path).cloned(),
                                app.cache_manager.folder_preview_loading.contains(&file.path),
                                app.metadata_loading.contains(&file.path),
                                folder_size,
                                is_folder_size_loading,
                                app.is_recycle_bin_view,
                                &mut app.item_icon_loader,
                                &mut app.svg_icon_manager,
                                Some(frame),
                                is_owner,
                                app.cache_manager.is_failed(&file.path),
                            );

                            if let Some(act) = action {
                                match act {
                                    PreviewPanelAction::RequestPlay(path) => {
                                        use crate::ui::components::media_preview::MediaPreview;
                                        use crate::ui::components::MpvPreview;
                                        
                                        // TAKE OVER: Stop and drop existing player if any
                                        if let Some(MediaPreview::Video(ref mut old_player)) = app.media_preview {
                                            old_player.pause();
                                            // Dropping app.media_preview will stop the previous player
                                        }
                                        app.media_preview = None;

                                        // Take ownership and start new player
                                        let mut player = MpvPreview::new(path);
                                        player.play_on_init = true; // Start playing as soon as initialized
                                        player.show_player = true;  // Ensure player is visible immediately
                                        app.media_preview = Some(MediaPreview::Video(player));
                                        app.media_preview_owner_tab_id = Some(tab_id);
                                        
                                        // Final sync: hide/show correctly
                                        app.update_video_visibility();
                                    }
                                    PreviewPanelAction::RefreshThumbnail(path) => {
                                        // Clear all caches to allow retry
                                        app.disk_cache.remove_cache_for_path(&path);
                                        app.cache_manager.texture_cache.pop(&path);
                                        app.cache_manager.loading_set.remove(&path);
                                        // Clear failure cache so it will be retried
                                        crate::workers::thumbnail_worker::clear_failure_cache(&path);
                                        app.request_thumbnail_load(path, 512);
                                        app.notifications.push(
                                            crate::application::AppNotification::info("Recarregando thumbnail...".to_string()),
                                        );
                                    }
                                    PreviewPanelAction::LoadFolderPreview(path) => {
                                        if app.cache_manager.folder_preview_loading.len() < 30 {
                                            app.cache_manager.folder_preview_loading.insert(path.clone());
                                            let _ = app.folder_preview_sender.send(path);
                                        }
                                    }
                                    PreviewPanelAction::CalculateFolderSize(path) => {
                                        app.folder_size_loading.insert(path.clone());
                                        let _ = app.folder_size_req_sender.send(path);
                                    }
                                }
                            }
                        } else {
                            ui.vertical_centered(|ui| {
                                ui.add_space(100.0);
                                ui.label("Nenhum item selecionado");
                                ui.label("Selecione algo para ver detalhes");
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
    let left_width = app.sidebar_left_width.clamp(LEFT_SIDEBAR_MIN, LEFT_SIDEBAR_MAX);
    let left_handle_rect = egui::Rect::from_min_size(
        egui::pos2(left_width - RESIZE_HANDLE_WIDTH / 2.0, tab_bar_height),
        egui::vec2(RESIZE_HANDLE_WIDTH, screen.height() - tab_bar_height),
    );
    
    egui::Area::new(egui::Id::new("left_sidebar_resize"))
        .fixed_pos(left_handle_rect.min)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let response = ui.allocate_rect(
                left_handle_rect,
                egui::Sense::drag(),
            );
            
            // Set cursor on hover/drag
            if response.hovered() || response.dragged() {
                ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            
            // Update width on drag
            if response.dragged() {
                let delta = response.drag_delta().x;
                app.sidebar_left_width = (app.sidebar_left_width + delta)
                    .clamp(LEFT_SIDEBAR_MIN, LEFT_SIDEBAR_MAX);
            }
        });
    
    // Right sidebar resize handle (left edge of right sidebar) - only if panel is visible
    if app.show_preview_panel {
        let right_width = app.sidebar_right_width.clamp(RIGHT_SIDEBAR_MIN, RIGHT_SIDEBAR_MAX);
        let right_handle_x = screen.width() - right_width - RESIZE_HANDLE_WIDTH / 2.0;
        let right_handle_rect = egui::Rect::from_min_size(
            egui::pos2(right_handle_x, tab_bar_height),
            egui::vec2(RESIZE_HANDLE_WIDTH, screen.height() - tab_bar_height),
        );
        
        egui::Area::new(egui::Id::new("right_sidebar_resize"))
            .fixed_pos(right_handle_rect.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let response = ui.allocate_rect(
                    right_handle_rect,
                    egui::Sense::drag(),
                );
                
                // Set cursor on hover/drag
                if response.hovered() || response.dragged() {
                    ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                
                // Update width on drag (note: dragging LEFT increases right panel width)
                if response.dragged() {
                    let delta = -response.drag_delta().x; // Inverted for right panel
                    app.sidebar_right_width = (app.sidebar_right_width + delta)
                        .clamp(RIGHT_SIDEBAR_MIN, RIGHT_SIDEBAR_MAX);
                }
            });
    }
}

fn calculate_effective_file(app: &ImageViewerApp) -> Option<FileEntry> {
    if let Some(file) = app.selected_file.clone() {
        // PERFORMANCE FIX: NEVER call path.exists() in render loop!
        // On HDD with video playing, this causes I/O spikes every frame.
        // File existence is validated on:
        // 1. Selection (when user clicks)
        // 2. File system watcher events (auto-refresh)
        // Trust the cached state - it's updated by the file watcher.
        Some(file)
    } else if app.is_recycle_bin_view {
        Some(FileEntry {
            path: PathBuf::from("Lixeira"),
            name: "Lixeira".to_string(),
            is_dir: true,
            size: 0,
            modified: 0,
            folder_cover: None,
            drive_info: None,
            sync_status: SyncStatus::None,
            deletion_date: None,
            recycle_original_path: None,
        })
    } else if app.is_computer_view {
        // "Este Computador" - show drive count info
        Some(FileEntry {
            path: PathBuf::from("Este Computador"),
            name: "Este Computador".to_string(),
            is_dir: true,
            size: app.disks.len() as u64, // Store drive count in size field
            modified: 0,
            folder_cover: None,
            drive_info: None,
            sync_status: SyncStatus::None,
            deletion_date: None,
            recycle_original_path: None,
        })
    } else {
        let path = std::path::PathBuf::from(&app.current_path);
        let mut entry = FileEntry::from_path(path.clone(), true);
        if path.to_string_lossy().len() <= 3 && path.to_string_lossy().contains(':') {
            use crate::infrastructure::windows::get_volume_info;
            let vol = get_volume_info(&app.current_path);
            let drive_type = windows_infra::detect_drive_type(&app.current_path);
            let label = app.disks.iter().find(|(p, _)| {
                p.starts_with(&app.current_path) || app.current_path.starts_with(p)
            }).map(|(_, l)| l.clone()).unwrap_or_else(|| app.current_path.clone());
            entry.name = label;
            entry.drive_info = Some(crate::domain::file_entry::DriveInfo {
                file_system: vol.file_system,
                total_space: vol.total_space,
                free_space: vol.free_space,
                drive_type,
            });
        } else {
            entry.name = path.file_name().map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| app.current_path.clone());
        }
        Some(entry)
    }
}

fn render_central_panel_layout(app: &mut ImageViewerApp, ctx: &egui::Context) {
    egui::CentralPanel::default().show(ctx, |ui| {
        if app.is_loading_folder && app.items.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.spinner();
                ui.label("Carregando...");
            });
        } else if app.items.is_empty() {
            let response = ui.centered_and_justified(|ui| {
                ui.label("Pasta vazia");
            }).response.on_hover_cursor(egui::CursorIcon::Default);

            // Handle context menu on empty area
            let interact_response = ui.interact(response.rect, ui.id().with("empty_bg"), egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::Default); // Force cursor on the interaction rect
            
            if interact_response.secondary_clicked() {
                app.context_menu.target_paths.clear(); 
                
                // Use current path for shell menu
                let paths = if app.is_recycle_bin_view {
                    vec![]
                } else {
                    vec![std::path::PathBuf::from(&app.current_path)] 
                };
                
                // Prepare state
                let pos = ui.input(|i| i.pointer.hover_pos().unwrap_or_default());
                let right_bound = ui.available_rect_before_wrap().right();
                
                // Set state first
                app.context_menu.open(pos, right_bound, None, paths.clone(), true);
                
                // Then populate items
                app.populate_context_menu(ui.ctx(), &paths, true, None);
            }
        } else {
            match app.view_mode {
                ViewMode::Grid => app.render_grid_view(ui),
                ViewMode::List => app.render_list_view(ui),
            }

            if ui.input(|i| i.key_pressed(egui::Key::F2)) {
                if let Some(idx) = app.selected_item {
                    if let Some(item) = app.items.get(idx) {
                        app.renaming_state = Some((idx, item.name.clone()));
                        app.focus_rename = true;
                    }
                }
            }

            if app.is_loading_folder {
                let rect = ui.max_rect();
                let spinner_rect = egui::Rect::from_min_size(
                    rect.right_bottom() - egui::vec2(24.0, 24.0),
                    egui::vec2(16.0, 16.0),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(spinner_rect), |ui| {
                    ui.spinner();
                });
            }
        }
    });
}

