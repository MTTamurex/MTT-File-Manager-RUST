use crate::app::ImageViewerApp;
use crate::domain::special_paths::COMPUTER_VIEW_ID;
use eframe::egui;

pub(crate) fn render_tab_bar_layer(
    app: &mut ImageViewerApp,
    ctx: &egui::Context,
    frame: &mut eframe::Frame,
) {
    egui::TopBottomPanel::top("tab_bar_panel")
        .show_separator_line(false)
        .exact_height(36.0)
        .frame(egui::Frame {
            fill: if ctx.style().visuals.dark_mode {
                egui::Color32::from_rgb(30, 30, 30)
            } else {
                egui::Color32::from_rgb(230, 230, 230)
            },
            ..Default::default()
        })
        .show(ctx, |ui| {
            use crate::ui::components::media_preview::MediaPreview;
            use crate::ui::tab_bar::{render_tab_bar, TabBarAction};

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
                app.is_item_dragging,
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
                    if current_path == COMPUTER_VIEW_ID {
                        app.setup_computer_view();
                    }
                    app.sync_to_tab();
                    app.update_video_visibility();
                }
                TabBarAction::CloseTab(idx) => {
                    log::debug!(
                        "[DEBUG] Closing Tab index: {}. Active was: {}",
                        idx, app.tab_manager.active_tab
                    );

                    if let Some(tab) = app.tab_manager.tabs.get(idx) {
                        let tab_id = tab.id;
                        if app.media_preview_owner_tab_id == Some(tab_id) {
                            log::debug!("[DEBUG] Closing tab owns media player. Destroying player.");
                            app.destroy_media_preview();
                        }
                    }

                    let closing_active_tab = idx == app.tab_manager.active_tab;

                    if app.tab_manager.close_tab(idx) {
                        log::debug!("[DEBUG] Last tab closed. Closing app.");
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    } else {
                        if closing_active_tab {
                            log::debug!(
                                "[DEBUG] Active tab closed. Switching to new active tab index: {}",
                                app.tab_manager.active_tab
                            );
                            app.sync_from_tab();
                        } else {
                            log::debug!(
                                "[DEBUG] Background tab closed. current active index adjusted to: {}. Saving live state to it.",
                                app.tab_manager.active_tab
                            );
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
