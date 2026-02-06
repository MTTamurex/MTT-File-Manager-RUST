use eframe::egui;
use crate::app::ImageViewerApp;
use crate::domain::file_entry::{SortMode, ViewMode};
use crate::ui::app;
use crate::ui::theme;
use crate::ui::widgets;
use crate::infrastructure::windows::window_subclass::is_in_size_move;

#[cfg(debug_assertions)]
macro_rules! debug_log {
    ($($arg:tt)*) => { eprintln!($($arg)*) }
}

#[cfg(not(debug_assertions))]
macro_rules! debug_log {
    ($($arg:tt)*) => {}
}

impl eframe::App for ImageViewerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Check if window is being resized/dragged (for UI optimization)
        let is_resizing = is_in_size_move();

        // 1. Initial validation
        if self.startup_tick == 0 {
            // Check for loaded fonts
            if let Some(rx) = &self.font_loader_rx {
                if let Ok(fonts) = rx.try_recv() {
                    ctx.set_fonts(fonts);
                    self.font_loader_rx = None; // Disable loader once done
                    ctx.request_repaint(); // Force refresh with new fonts
                }
            }

            // NOTE: Removed path.exists() check here because it can BLOCK indefinitely
            // on OneDrive cloud-only files, causing UI freeze. The file selection will
            // be cleared naturally when the user navigates away or refreshes the folder.
            // If we need this check, it should be done asynchronously in a worker thread.
        }

        // 2. Lifecycle: Startup sequence & window state tracking
        app::lifecycle::handle_startup_sequence(self, ctx);
        app::lifecycle::track_window_state(self, ctx);
        let frame_ms = (ctx.input(|i| i.stable_dt) * 1000.0) as f32;
        if frame_ms > 0.0 {
            if self.frame_time_avg_ms <= 0.0 {
                self.frame_time_avg_ms = frame_ms;
            } else {
                self.frame_time_avg_ms = self.frame_time_avg_ms * 0.9 + frame_ms * 0.1;
            }
            if self.frame_time_peak_ms <= 0.0 {
                self.frame_time_peak_ms = frame_ms;
            } else {
                self.frame_time_peak_ms *= 0.95;
                if frame_ms > self.frame_time_peak_ms {
                    self.frame_time_peak_ms = frame_ms;
                }
            }
            self.fps_avg = if self.frame_time_avg_ms > 0.0 {
                1000.0 / self.frame_time_avg_ms
            } else {
                0.0
            };
        }

        // 3. Infrastructure updates (skip heavy processing during resize)
        self.ensure_window_handle(frame);
        if !is_resizing {
            self.process_incoming_messages(ctx);
            self.refresh_drives_if_needed();
        }
        self.ensure_folder_icon(ctx);
        self.ensure_computer_icon(ctx);
        self.item_icon_loader.ensure_folder_icon(ctx);

        // 4. Input: Keyboard shortcuts (resize borders handled by native subclass)
        if !is_resizing {
            app::input::handle_input(self, ctx);
        }

        // 5. Layout: Status Bar (Bottom) - lightweight, always render
        render_status_bar_layer(self, ctx);

        // 6. Layout: Tab Bar (Top 1) - lightweight, always render
        render_tab_bar_layer(self, ctx, frame);

        // 7. Layout: Toolbar (Top 2) - lightweight, always render
        render_toolbar_layer(self, ctx);

        // 7b. Layout: Secondary Toolbar (Top 3) - lightweight, always render
        render_secondary_toolbar_layer(self, ctx);

        // 8-11. Heavy operations: Skip during resize for smooth animation
        if is_resizing {
            // Simplified placeholder during resize
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(if ctx.style().visuals.dark_mode {
                    egui::Color32::from_rgb(45, 45, 45)
                } else {
                    egui::Color32::WHITE
                }))
                .show(ctx, |_ui| {
                    // Empty panel - just fill with background color
                });
        } else {
            // 8. Layout: Main Panels (Sidebar, Preview, Central)
            app::panels::render_panels(self, ctx, frame);

            // 9. Operations: Context Menu (Rendering & Actions)
            app::menu_handler::handle_context_menu(self, ctx);

            // 10. Operations: Resize borders (on top) - REMOVED, handled by native subclass
            // app::input::handle_resize_borders(self, ctx);

            // 11. Virtual drive settings modal
            if self.show_virtual_drive_settings {
                self.show_virtual_drive_settings = crate::ui::components::virtual_drive_settings::render_virtual_drive_settings(
                    ctx,
                    self.show_virtual_drive_settings,
                );
            }

            // 12. Notifications
            app::notifications::render_notifications(self, ctx);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        app::lifecycle::handle_exit(self);
    }
}

// Helper layers to keep update() very clean
fn render_status_bar_layer(app: &mut ImageViewerApp, ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("status_bar")
        .exact_height(24.0)
        .show(ctx, |ui| {
            use crate::ui::status_bar::{render_status_bar, StatusBarAction};
            let action = render_status_bar(
                ui,
                &mut app.is_loading_folder,
                app.total_items,
                &mut app.view_mode,
                &mut app.sort_mode,
                &mut app.sort_descending,
                &mut app.folders_position,
                &app.cache_manager.texture_cache,
                app.frame_time_avg_ms,
                app.frame_time_peak_ms,
                app.fps_avg,
                app.upload_budget_ms,
                app.is_computer_view,
            );
            match action {
                StatusBarAction::SortChanged => {
                    // Save to the correct field based on context
                    if app.is_computer_view {
                        app.sort_mode_computer = app.sort_mode;
                    } else {
                        app.sort_mode_normal = app.sort_mode;
                    }
                    app.sort_items();
                    app.save_preferences();
                }
                StatusBarAction::OpenVirtualDriveSettings => {
                    app.show_virtual_drive_settings = true;
                }
                _ => {}
            }
        });
}

fn render_tab_bar_layer(app: &mut ImageViewerApp, ctx: &egui::Context, frame: &mut eframe::Frame) {
    egui::TopBottomPanel::top("tab_bar_panel")
        .show_separator_line(false)
        .exact_height(36.0)
        .frame(egui::Frame {
            fill: if ctx.style().visuals.dark_mode {
                egui::Color32::from_rgb(32, 32, 32)
            } else {
                egui::Color32::from_rgb(243, 243, 243)
            },
            ..Default::default()
        })
        .show(ctx, |ui| {
            use crate::ui::tab_bar::{render_tab_bar, TabBarAction};
            use crate::ui::components::media_preview::MediaPreview;

            let (playing, muted) = if let Some(MediaPreview::Video(player)) = &app.media_preview {
                let state = player.get_state();
                (state.is_playing, state.is_muted)
            } else {
                (false, false)
            };

            let action = render_tab_bar(
                ui,
                &app.tab_manager,
                &mut app.svg_icon_manager,
                frame,
                app.cache_manager.computer_icon.as_ref(),
                &mut app.item_icon_loader,
                app.media_preview_owner_tab_id,
                playing,
                muted,
            );

            match action {
                TabBarAction::ToggleMute(_idx) => {
                    if let Some(MediaPreview::Video(player)) = &mut app.media_preview {
                        player.toggle_mute();
                    }
                }
                TabBarAction::SwitchTab(idx) => {
                    app.sync_to_tab();
                    app.tab_manager.switch_to(idx);
                    app.sync_from_tab();
                    // Control player visibility based on owner
                    app.update_video_visibility();
                }
                TabBarAction::NewTab => {
                    let prev_view_mode = app.view_mode;
                    let prev_sort_mode = app.sort_mode;
                    let prev_sort_descending = app.sort_descending;
                    let prev_folders_position = app.folders_position;
                    app.sync_to_tab();
                    let current_path = app.tab_manager.active().path.clone();
                    app.tab_manager.new_tab_at(&current_path);
                    let active = app.tab_manager.active_mut();
                    active.view_mode = prev_view_mode;
                    active.sort_mode = prev_sort_mode;
                    active.sort_descending = prev_sort_descending;
                    active.folders_position = prev_folders_position;
                    app.sync_from_tab();
                    if current_path == "Este Computador" {
                        app.setup_computer_view();
                    }
                    app.sync_to_tab();
                    // Control player visibility based on owner
                    app.update_video_visibility();
                }
                TabBarAction::CloseTab(idx) => {
                    eprintln!("[DEBUG] Closing Tab index: {}. Active was: {}", idx, app.tab_manager.active_tab);
                    
                    // CLEANUP LOGIC: If the tab being closed is the owner of the media player, destroy the player.
                    if let Some(tab) = app.tab_manager.tabs.get(idx) {
                        let tab_id = tab.id;
                        if app.media_preview_owner_tab_id == Some(tab_id) {
                            eprintln!("[DEBUG] Closing tab owns media player. Destroying player.");
                            if let Some(crate::ui::components::media_preview::MediaPreview::Video(ref mut wv)) = app.media_preview {
                                wv.pause();
                            }
                            app.media_preview = None;
                            app.media_preview_owner_tab_id = None;
                            app.ui_ctx.request_repaint();
                        }
                    }

                    // Check if we are closing the currently active tab
                    let closing_active_tab = idx == app.tab_manager.active_tab;

                    if app.tab_manager.close_tab(idx) {
                        eprintln!("[DEBUG] Last tab closed. Closing app.");
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    } else {
                        if closing_active_tab {
                            // We closed the active tab, so we MUST switch context to the new active tab
                            eprintln!("[DEBUG] Active tab closed. Switching to new active tab index: {}", app.tab_manager.active_tab);
                            app.sync_from_tab();
                        } else {
                            // We closed a background tab. The user is still looking at the same logical tab.
                            // We should NOT reload state (sync_from_tab) because the Saved State might be stale (e.g., pending items).
                            // Instead, we should SAVE the current fresh Live State to the new slot of the active tab.
                            eprintln!("[DEBUG] Background tab closed. current active index adjusted to: {}. Saving live state to it.", app.tab_manager.active_tab);
                            app.sync_to_tab();
                        }

                        app.update_video_visibility();
                    }
                }
                TabBarAction::CloseApp => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                TabBarAction::ToggleMaximize => {
                    let is_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_maximized));
                }
                TabBarAction::Minimize => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                TabBarAction::None => {}
            }
        });
}

fn render_toolbar_layer(app: &mut ImageViewerApp, ctx: &egui::Context) {
    egui::TopBottomPanel::top("nav_bar")
        .show_separator_line(true)
        .exact_height(46.0) // Increased height like Windows Explorer
        .frame(egui::Frame {
            fill: if ctx.style().visuals.dark_mode {
                egui::Color32::from_rgb(45, 45, 45)
            } else {
                egui::Color32::from_rgb(243, 243, 243) // Same as active tab (Windows Explorer style)
            },
            inner_margin: egui::Margin { left: 8, right: 8, top: 7, bottom: 7 }, // Padding to center content in taller bar
            ..Default::default()
        })
        .show(ctx, |ui| {
            use crate::ui::toolbar::{render_toolbar, ToolbarAction};
            use crate::domain::file_entry::ViewMode;

            let action = render_toolbar(
                ui,
                &app.current_path,
                &mut app.path_input,
                &mut app.is_address_editing,
                &mut app.search_query,
                &app.navigation,
                app.view_mode,
                app.sort_mode,
                app.sort_descending,
                &mut app.thumbnail_size,
                app.show_preview_panel,
                app.renaming_state.is_some(),
                app.cache_manager.computer_icon.as_ref(),
                &mut app.svg_icon_manager,
            );

            if let Some(act) = action {
                match act {
                    ToolbarAction::GoBack => app.go_back(),
                    ToolbarAction::GoForward => app.go_forward(),
                    ToolbarAction::GoUp => app.go_up_one_level(),
                    ToolbarAction::Refresh => app.trigger_manual_refresh(),
                    ToolbarAction::CreateFolder => app.create_new_folder(),
                    ToolbarAction::NavigateToComputer => app.navigate_to_computer(),
                    ToolbarAction::NavigateToRecycleBin => app.navigate_to_recycle_bin(),
                    ToolbarAction::ToggleViewMode => {
                        if app.view_mode == ViewMode::List {
                            app.view_mode = ViewMode::Grid;
                        } else {
                            app.view_mode = ViewMode::List;
                        }
                        debug_log!("[VIEW-MODE] Toolbar toggle -> {:?} (tab={})", app.view_mode, app.tab_manager.active_tab);
                    }
                    ToolbarAction::TogglePreviewPanel => {
                        app.show_preview_panel = !app.show_preview_panel;
                        app.update_video_visibility();
                    }
                    ToolbarAction::ChangeSortMode(mode) => {
                        app.sort_mode = mode;
                        if app.is_computer_view {
                            app.sort_mode_computer = mode;
                        } else {
                            app.sort_mode_normal = mode;
                        }
                        app.sort_items();
                        app.save_preferences();
                    }
                    ToolbarAction::ToggleSortDescending => {
                        app.sort_descending = !app.sort_descending;
                        app.sort_items();
                        app.save_preferences();
                    }
                    ToolbarAction::Search(_query) => app.filter_items(),
                    ToolbarAction::Navigate(path) => app.navigate_to(&path),
                    ToolbarAction::StartAddressEdit => {
                        app.path_input = app.current_path.clone();
                        app.is_address_editing = true;
                    }
                    ToolbarAction::CommitPathInput(path) => {
                        // CRITICAL FIX: Use fast_path_exists() instead of path.exists()
                        // path.exists() uses CreateFileW which triggers OneDrive file recall,
                        // blocking the UI thread for 30-60s on cloud-only files.
                        if crate::infrastructure::onedrive::fast_path_exists(std::path::Path::new(&path)) {
                            app.navigate_to(&path);
                            app.is_address_editing = false;
                        } else {
                            app.path_input = app.current_path.clone();
                            app.is_address_editing = false;
                        }
                    }
                    ToolbarAction::CancelPathInput => {
                        app.is_address_editing = false;
                        app.path_input = app.current_path.clone();
                    }
                    _ => {}
                }
            }
        });
}

fn render_secondary_toolbar_layer(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let separator_color = if ctx.style().visuals.dark_mode {
        egui::Color32::from_rgb(80, 80, 80)
    } else {
        egui::Color32::from_rgb(210, 210, 210)
    };
    
    egui::TopBottomPanel::top("secondary_nav_bar")
        .show_separator_line(false)
        .exact_height(46.0) // Same height as main toolbar
        .frame(egui::Frame {
            fill: if ctx.style().visuals.dark_mode {
                egui::Color32::from_rgb(45, 45, 45)
            } else {
                egui::Color32::WHITE
            },
            inner_margin: egui::Margin { left: 8, right: 8, top: 7, bottom: 7 },
            ..Default::default()
        })
        .show(ctx, |ui| {
            // Draw bottom separator line manually
            let rect = ui.max_rect();
            ui.painter().hline(
                rect.x_range(),
                rect.bottom(),
                egui::Stroke::new(1.0, separator_color),
            );
            
            // Internal enum to defer actions and avoid borrow checker conflicts
            enum SecAction {
                None,
                Cut,
                Copy,
                Paste,
                Rename,
                CreateFolder,
                Delete,
            }
            let mut action = SecAction::None;

            // Centered horizontal layout
            ui.horizontal(|ui| {
                // Calculate approximate content width to center it
                // 6 icons (28px) + sort button + combobox (~70px) + 2 view icons + slider (80px) + zoom label + separators + spacing
                let content_width = 6.0 * 28.0 + 30.0 + 110.0 + 2.0 * 28.0 + 80.0 + 80.0 + 3.0 * 8.0 + 16.0 * 12.0;
                let available = ui.available_width();
                let left_pad = ((available - content_width) / 2.0).max(0.0);
                ui.add_space(left_pad);
                
                ui.spacing_mut().item_spacing = egui::vec2(12.0, 0.0);

                let icon_size = egui::vec2(28.0, 28.0); // Consistent button size
                
                // --- Logic for Enablement ---
                // Calculated BEFORE the mutable borrow of svg_icon_manager
                let is_drive_selected = app.selected_file.as_ref().map_or(false, |f| f.drive_info.is_some());
                let has_selection = (app.selected_file.is_some() || !app.multi_selection.is_empty()) && !is_drive_selected;
                let can_rename = app.multi_selection.len() <= 1 && (app.multi_selection.len() == 1 || app.selected_file.is_some());
                let can_paste = app.clipboard.has_content() && !is_drive_selected;
                let can_create_folder = !app.is_computer_view && !app.is_recycle_bin_view;

                // Colors
                let icon_color = if ui.visuals().dark_mode {
                    [220, 220, 220, 255]
                } else {
                    [60, 60, 60, 255]
                };
                let disabled_color = [128, 128, 128, 180];

                {
                    // Borrow manager for the closure scope
                    let svg_manager = &mut app.svg_icon_manager;

                    // --- Helper Closure for Rendering Buttons ---
                    let mut render_btn = |icon_name: &str, enabled: bool, tooltip: &str| -> bool {
                        let color = if enabled { icon_color } else { disabled_color };
                        let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
                        let (rect, response) = ui.allocate_exact_size(icon_size, sense);

                        if enabled && response.hovered() {
                            let bg_color = if ui.visuals().dark_mode {
                                theme::color_dark_hover()
                            } else {
                                theme::color_hover()
                            };
                            ui.painter().rect_filled(rect, 6.0, bg_color);
                        }

                        if let Some(texture) = svg_manager.get_icon(
                            ui.ctx(),
                            icon_name,
                            32,
                            color,
                        ) {
                            let display_size = if icon_name == "folder_new" { 18.0 } else { 16.0 };
                            let icon_rect = egui::Rect::from_center_size(
                                rect.center(),
                                egui::vec2(display_size, display_size),
                            );
                            ui.painter().image(
                                texture.id(),
                                icon_rect,
                                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE,
                            );
                        } else {
                            let fallback = icon_name.chars().next().unwrap_or('?').to_string();
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                fallback,
                                egui::FontId::proportional(12.0),
                                egui::Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]),
                            );
                        }

                        let response = if enabled {
                            response.on_hover_cursor(egui::CursorIcon::PointingHand)
                        } else {
                            response
                        };

                        if enabled {
                            response.on_hover_text(tooltip).clicked()
                        } else {
                            response.on_hover_text(format!("{} (Desabilitado)", tooltip));
                            false
                        }
                    };

                    // 1. Cut
                    if render_btn("cut", has_selection, "Recortar (Ctrl+X)") {
                        action = SecAction::Cut;
                    }

                    // 2. Copy
                    if render_btn("copy", has_selection, "Copiar (Ctrl+C)") {
                        action = SecAction::Copy;
                    }

                    // 3. Paste
                    if render_btn("paste", can_paste, "Colar (Ctrl+V)") {
                        action = SecAction::Paste;
                    }

                    // 4. Rename (allowed for drives too — renames volume label)
                    if render_btn("rename", can_rename, "Renomear (F2)") {
                        action = SecAction::Rename;
                    }

                    // 5. Create Folder
                    if render_btn("folder_new", can_create_folder, "Criar Nova Pasta (Ctrl+Shift+N)") {
                        action = SecAction::CreateFolder;
                    }

                    // 6. Delete
                    if render_btn("delete", has_selection, "Excluir (Del)") {
                        action = SecAction::Delete;
                    }
                }

                ui.separator();

                // Ordenação (dropdown + asc/desc)
                let sort_symbol = if app.sort_descending { "↓" } else { "↑" };

                ui.scope(|ui| {
                    let hover_color = if ui.visuals().dark_mode {
                        theme::color_dark_hover()
                    } else {
                        theme::color_hover()
                    };

                    ui.visuals_mut().widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
                    ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                    ui.visuals_mut().widgets.inactive.fg_stroke = egui::Stroke::NONE;
                    ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;

                    ui.visuals_mut().widgets.hovered.bg_fill = hover_color;
                    ui.visuals_mut().widgets.hovered.fg_stroke = egui::Stroke::NONE;
                    ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;

                    if ui
                        .add(egui::Button::new(egui::RichText::new(sort_symbol).color(egui::Color32::BLACK)))
                        .on_hover_text("Inverter Ordem")
                        .clicked()
                    {
                        app.sort_descending = !app.sort_descending;
                        app.sort_items();
                        app.save_preferences();
                    }
                });

                ui.scope(|ui| {
                    let hover_color = if ui.visuals().dark_mode {
                        theme::color_dark_hover()
                    } else {
                        theme::color_hover()
                    };

                    let black_stroke = egui::Stroke::new(1.0, egui::Color32::BLACK);

                    ui.visuals_mut().widgets.noninteractive.bg_fill = egui::Color32::WHITE;
                    ui.visuals_mut().widgets.noninteractive.fg_stroke = black_stroke;
                    ui.visuals_mut().widgets.noninteractive.bg_stroke = egui::Stroke::NONE;

                    ui.visuals_mut().widgets.inactive.bg_fill = egui::Color32::WHITE;
                    ui.visuals_mut().widgets.inactive.fg_stroke = black_stroke;
                    ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;

                    ui.visuals_mut().widgets.hovered.bg_fill = hover_color;
                    ui.visuals_mut().widgets.hovered.fg_stroke = black_stroke;
                    ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;

                    ui.visuals_mut().widgets.active.bg_fill = hover_color;
                    ui.visuals_mut().widgets.active.fg_stroke = black_stroke;
                    ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::NONE;

                    ui.visuals_mut().override_text_color = Some(egui::Color32::BLACK);

                    egui::ComboBox::from_id_salt("sort_mode_secondary")
                        .selected_text(match app.sort_mode {
                            SortMode::Name => "Nome",
                            SortMode::Date => "Data",
                            SortMode::Size => "Tamanho",
                            SortMode::Type => "Tipo",
                            SortMode::DriveTotalSpace => "Espaço Total",
                            SortMode::DriveFreeSpace => "Espaço Livre",
                        })
                        .show_ui(ui, |ui| {
                            // Opção Nome sempre disponível
                            if ui.selectable_value(&mut SortMode::Name, app.sort_mode, "Nome").clicked() {
                                app.sort_mode = SortMode::Name;
                                if app.is_computer_view {
                                    app.sort_mode_computer = SortMode::Name;
                                } else {
                                    app.sort_mode_normal = SortMode::Name;
                                }
                                app.sort_items();
                                app.save_preferences();
                            }
                            
                            // Opções específicas para Computer View
                            if app.is_computer_view {
                                if ui.selectable_value(&mut SortMode::DriveTotalSpace, app.sort_mode, "Espaço Total").clicked() {
                                    app.sort_mode = SortMode::DriveTotalSpace;
                                    app.sort_mode_computer = SortMode::DriveTotalSpace;
                                    app.sort_items();
                                    app.save_preferences();
                                }
                                if ui.selectable_value(&mut SortMode::DriveFreeSpace, app.sort_mode, "Espaço Livre").clicked() {
                                    app.sort_mode = SortMode::DriveFreeSpace;
                                    app.sort_mode_computer = SortMode::DriveFreeSpace;
                                    app.sort_items();
                                    app.save_preferences();
                                }
                            } else {
                                // Opções para visualização normal (não Computer View)
                                if ui.selectable_value(&mut SortMode::Date, app.sort_mode, "Data").clicked() {
                                    app.sort_mode = SortMode::Date;
                                    app.sort_mode_normal = SortMode::Date;
                                    app.sort_items();
                                    app.save_preferences();
                                }
                                if ui.selectable_value(&mut SortMode::Size, app.sort_mode, "Tamanho").clicked() {
                                    app.sort_mode = SortMode::Size;
                                    app.sort_mode_normal = SortMode::Size;
                                    app.sort_items();
                                    app.save_preferences();
                                }
                                if ui.selectable_value(&mut SortMode::Type, app.sort_mode, "Tipo").clicked() {
                                    app.sort_mode = SortMode::Type;
                                    app.sort_mode_normal = SortMode::Type;
                                    app.sort_items();
                                    app.save_preferences();
                                }
                            }
                        });
                });

                ui.separator();

                // View mode
                {
                    let svg_manager = &mut app.svg_icon_manager;
                    if widgets::toggle_icon_button(
                        ui,
                        svg_manager,
                        theme::ICON_LIST,
                        matches!(app.view_mode, ViewMode::List),
                        "Lista",
                    )
                    .clicked()
                    {
                        if !matches!(app.view_mode, ViewMode::List) {
                            app.view_mode = ViewMode::List;
                        }
                    }

                    if widgets::toggle_icon_button(
                        ui,
                        svg_manager,
                        theme::ICON_GRID,
                        matches!(app.view_mode, ViewMode::Grid),
                        "Grade",
                    )
                    .clicked()
                    {
                        if !matches!(app.view_mode, ViewMode::Grid) {
                            app.view_mode = ViewMode::Grid;
                        }
                    }
                }

                ui.separator();

                // Zoom
                ui.add_sized(
                    egui::vec2(80.0, 20.0),
                    egui::Slider::new(&mut app.thumbnail_size, 64.0..=256.0).show_value(false),
                );
                ui.label("Zoom");
            });

            // Execute deferred action
            match action {
                SecAction::Cut => app.command_cut(Option::from(app.selected_item)),
                SecAction::Copy => app.command_copy(Option::from(app.selected_item)),
                SecAction::Paste => app.command_paste(None),
                SecAction::Rename => {
                     if let Some(idx) = app.selected_item {
                        if let Some(item) = app.items.get(idx) {
                            app.renaming_state = Some((idx, item.name.clone()));
                            app.focus_rename = true;
                        }
                    }
                },
                SecAction::CreateFolder => app.create_new_folder(),
                SecAction::Delete => {
                    let mut targets = Vec::new();
                    if app.multi_selection.is_empty() {
                         if let Some(idx) = app.selected_item {
                             if let Some(item) = app.items.get(idx) {
                                 targets.push(item.path.clone());
                             }
                         }
                    } else {
                        targets.extend(app.multi_selection.iter().cloned());
                    }

                    if !targets.is_empty() {
                        app.delete_with_shell_for_paths(&targets);
                    }
                },
                SecAction::None => {}
            }
        });
}
