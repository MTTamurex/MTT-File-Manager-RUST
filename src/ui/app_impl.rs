use eframe::egui;
use crate::app::ImageViewerApp;
use crate::ui::app;
use crate::infrastructure::windows::window_subclass::is_in_size_move;

impl eframe::App for ImageViewerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Check if window is being resized/dragged (for UI optimization)
        let is_resizing = is_in_size_move();

        // 1. Initial validation
        if self.startup_tick == 0 {
            if let Some(ref file) = self.selected_file {
                if !file.path.exists() {
                    self.selected_file = None;
                    self.selected_thumbnail = None;
                    self.media_preview = None;
                    self.media_preview_owner_tab_id = None;
                    self.selected_metadata = None;
                }
            }
        }

        // 2. Lifecycle: Startup sequence & window state tracking
        app::lifecycle::handle_startup_sequence(self, ctx);
        app::lifecycle::track_window_state(self, ctx);

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

        // 8-11. Heavy operations: Skip during resize for smooth animation
        if is_resizing {
            // Simplified placeholder during resize
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(ctx.style().visuals.panel_fill))
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

            // 11. Notifications
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
                &mut app.thumbnail_size,
                &mut app.sort_mode,
                &mut app.sort_descending,
                &mut app.folders_position,
                &app.cache_manager.texture_cache,
            );
            match action {
                StatusBarAction::SortChanged => {
                    app.sort_items();
                    app.save_preferences();
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
                    app.sync_to_tab();
                    app.tab_manager.new_tab();
                    app.sync_from_tab();
                    app.setup_computer_view();
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
                    }
                    ToolbarAction::TogglePreviewPanel => {
                        app.show_preview_panel = !app.show_preview_panel;
                        app.update_video_visibility();
                    }
                    ToolbarAction::ChangeSortMode(mode) => {
                        app.sort_mode = mode;
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
                        if std::path::Path::new(&path).exists() {
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
