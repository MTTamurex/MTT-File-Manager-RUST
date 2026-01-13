use eframe::egui;
use std::path::PathBuf;
use crate::domain::file_entry::{FileEntry, SyncStatus, ViewMode};
use crate::infrastructure::windows as windows_infra;
use crate::app::ImageViewerApp;

pub fn render_panels(app: &mut ImageViewerApp, ctx: &egui::Context, _frame: &mut eframe::Frame) {
    // 1. Sidebar
    render_sidebar_panel(app, ctx);

    // 2. Preview Panel
    render_preview_panel_layout(app, ctx);

    // 3. Central Panel
    render_central_panel_layout(app, ctx);
}

fn render_sidebar_panel(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let sidebar_response = egui::SidePanel::left("sidebar")
        .min_width(150.0)
        .default_width(app.sidebar_left_width.max(150.0))
        .resizable(true)
        .show(ctx, |ui| {
            use crate::ui::sidebar::{render_sidebar, SidebarContext};

            let disks = app.disks.clone();
            let current_path = app.current_path.clone();
            let is_computer_view = app.is_computer_view;
            let computer_icon = app.cache_manager.computer_icon.clone();

            let mut ctx = SidebarContext {
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

            render_sidebar(ui, &mut ctx)
        });

    let is_minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
    let actual_panel_width = sidebar_response.response.rect.width();
    if !is_minimized
        && actual_panel_width > 100.0
        && (app.sidebar_left_width - actual_panel_width).abs() > 2.0
    {
        app.sidebar_left_width = actual_panel_width;
    }

    if let Some(action) = sidebar_response.inner {
        use crate::ui::sidebar::SidebarAction;
        match action {
            SidebarAction::NavigateTo(path) => app.navigate_to(&path),
            SidebarAction::NavigateToComputer => app.navigate_to_computer(),
            SidebarAction::NavigateToRecycleBin => app.navigate_to_recycle_bin(),
        }
    }
}

fn render_preview_panel_layout(app: &mut ImageViewerApp, ctx: &egui::Context) {
    if app.show_preview_panel {
        app.refresh_selected_metadata();

        let right_panel_response = egui::SidePanel::right("preview_panel")
            .resizable(true)
            .default_width(app.sidebar_right_width.max(250.0))
            .min_width(250.0)
            .max_width(500.0)
            .show(ctx, |ui| {
                use crate::ui::preview_panel::{render_preview_panel, PreviewPanelAction};

                egui::ScrollArea::vertical()
                    .id_salt("preview_scroll")
                    .show(ui, |ui| {
                        ui.set_max_width(ui.available_width());

                        let effective_file = calculate_effective_file(app);

                        if let Some(file) = effective_file {
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

                            let action = render_preview_panel(
                                ui,
                                &file,
                                app.selected_thumbnail.as_ref(),
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
                            );

                            if let Some(act) = action {
                                match act {
                                    PreviewPanelAction::RefreshThumbnail(path) => {
                                        app.disk_cache.remove_cache_for_path(&path);
                                        app.cache_manager.texture_cache.pop(&path);
                                        app.cache_manager.loading_set.remove(&path);
                                        let _ = app.thumbnail_req_sender.send((path, app.generation));
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

        let is_minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        let actual_panel_width = right_panel_response.response.rect.width();
        if !is_minimized
            && actual_panel_width > 200.0
            && (app.sidebar_right_width - actual_panel_width).abs() > 2.0
        {
            app.sidebar_right_width = actual_panel_width;
        }
    }
}

fn calculate_effective_file(app: &ImageViewerApp) -> Option<FileEntry> {
    if let Some(file) = app.selected_file.clone() {
        if app.is_recycle_bin_view || file.path.exists() {
            Some(file)
        } else {
            None
        }
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
        })
    } else if !app.is_computer_view {
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
    } else {
        None
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
            ui.centered_and_justified(|ui| {
                ui.label("Pasta vazia");
            });
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

        handle_central_panel_context_menu(app, ui);
    });
}

fn handle_central_panel_context_menu(app: &mut ImageViewerApp, ui: &mut egui::Ui) {
    if !app.context_menu.is_open && ui.input(|i| i.pointer.secondary_clicked()) {
        let pointer_pos = ui.ctx().pointer_latest_pos();
        let mut clicked_on_item = false;

        if let Some(pos) = pointer_pos {
            if app.view_mode == ViewMode::Grid && !app.items.is_empty() {
                let padding = 8.0;
                let item_w = app.thumbnail_size;
                let item_h = app.thumbnail_size + 20.0;
                let available_w = ui.available_width();
                let cols = ((available_w - padding) / (item_w + padding)).floor().max(1.0) as usize;

                let content_min = ui.min_rect().min;
                let relative_x = pos.x - content_min.x;
                let relative_y = pos.y - content_min.y;

                let col = (relative_x / (item_w + padding)).floor() as usize;
                let row = (relative_y / (item_h + padding)).floor() as usize;
                let index = row * cols + col;

                if index < app.items.len() {
                    clicked_on_item = true;
                }
            } else if app.view_mode == ViewMode::List && !app.items.is_empty() {
                let row_height = 24.0;
                let total_rows = app.items.len();
                let scroll_area_top = ui.min_rect().top();
                let relative_y = pos.y - scroll_area_top;

                let row = (relative_y / row_height).floor() as usize;
                if row < total_rows {
                    clicked_on_item = true;
                }
            }
        }

        if !clicked_on_item {
            let path = PathBuf::from(&app.current_path);
            let pointer_pos = ui.ctx().pointer_latest_pos().unwrap_or(egui::Pos2::ZERO);
            app.populate_context_menu(ui.ctx(), &path, true, None);
            app.context_menu.open(pointer_pos, None, Some(path), true);
        }
    }
}
