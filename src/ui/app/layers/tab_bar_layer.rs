use crate::app::ImageViewerApp;
use eframe::egui;

pub(crate) fn render_tab_bar_layer(
    app: &mut ImageViewerApp,
    root_ui: &mut egui::Ui,
    frame: &mut eframe::Frame,
) {
    let dark_mode = root_ui.visuals().dark_mode;
    let ctx = root_ui.ctx().clone();
    egui::Panel::top("tab_bar_panel")
        .show_separator_line(false)
        .exact_size(36.0)
        .frame(egui::Frame {
            fill: if dark_mode {
                crate::ui::theme::palette::background(
                    root_ui.ctx(),
                    egui::Color32::from_rgb(30, 30, 30),
                )
            } else {
                egui::Color32::from_rgb(230, 230, 230)
            },
            ..Default::default()
        })
        .show(root_ui, |ui| {
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
                &app.tag_definitions,
                &app.pinned_folders,
                &app.navigation_state.current_path,
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
                    app.save_preferences();
                }
                TabBarAction::MoveTab { from, gap } => {
                    app.sync_to_tab();
                    if app.tab_manager.move_tab_to_gap(from, gap) {
                        app.save_preferences();
                    }
                }
                TabBarAction::NewTab => {
                    app.create_default_main_tab();
                }
                TabBarAction::CloseTab(idx) => {
                    app.close_main_tab(idx);
                }
                TabBarAction::Context(index, action) => app.main_tab_action(index, action),
                TabBarAction::CloseApp => {
                    app.request_window_close(&ctx);
                }
                TabBarAction::ToggleMaximize => {
                    let is_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_maximized));
                }
                TabBarAction::Minimize => {
                    app.dismiss_context_menu();
                    crate::infrastructure::windows::window_subclass::freeze_layout(
                        app.layout.sidebar_left_width,
                        app.layout.sidebar_right_width,
                    );
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                TabBarAction::None => {}
            }
        });
}
