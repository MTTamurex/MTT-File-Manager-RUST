use crate::ui::components::MediaPreview;
use crate::ui::preview_panel::video_preview::controls::draw_video_controls;
use crate::ui::svg_icons::SvgIconManager;
use eframe::egui;

#[allow(clippy::too_many_arguments)]
pub fn render_fullscreen_video(
    ui: &mut egui::Ui,
    preview: &mut MediaPreview,
    svg_manager: &mut SvgIconManager,
    frame: Option<&eframe::Frame>,
    current_time: f64,
    duration: f64,
    volume: f32,
    is_muted: bool,
    is_playing: bool,
) {
    // Send fullscreen viewport command if not yet applied
    // (must be done here in the render loop, not in the button handler,
    // to ensure correct viewport transition — matches original working logic)
    if !preview.fullscreen_applied() {
        if preview.prev_app_maximized() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Maximized(false));
        }
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        preview.set_fullscreen_applied(true);
    }

    // Use viewport inner rect (actual drawable area)
    let screen_rect = ui
        .ctx()
        .input(|i| i.viewport().inner_rect)
        .unwrap_or_else(|| ui.ctx().screen_rect());

    egui::Area::new(egui::Id::new("video_fullscreen"))
        .fixed_pos(screen_rect.min)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            ui.set_min_size(screen_rect.size());
            ui.painter()
                .rect_filled(screen_rect, 0.0, egui::Color32::BLACK);

            let total_size = screen_rect.size();

            // Control area rect (always calculate for hover detection)
            let control_height_base = 75.0;
            let control_rect_hover = egui::Rect::from_min_size(
                egui::pos2(screen_rect.min.x, screen_rect.max.y - control_height_base),
                egui::vec2(total_size.x, control_height_base),
            );

            // Check if mouse is over control area and keep visible
            let mouse_over_controls = ui.input(|i| {
                i.pointer
                    .hover_pos()
                    .map(|p| control_rect_hover.contains(p))
                    .unwrap_or(false)
            });

            if mouse_over_controls {
                preview.reset_mouse_activity();
            }

            // Autohide logic
            let show_controls = preview.controls_active();
            let control_height = if show_controls { 75.0 } else { 0.0 };
            let video_height = total_size.y - control_height;

            let video_rect =
                egui::Rect::from_min_size(screen_rect.min, egui::vec2(total_size.x, video_height));

            // Allocate the full area with click detection
            let video_response = ui.allocate_exact_size(total_size, egui::Sense::click());

            // Double-click on video area to exit fullscreen
            if video_response.1.double_clicked() {
                preview.toggle_maximized();
                preview.set_fullscreen_applied(false);
                preview.set_forced_size(None); // Clear forced size when exiting fullscreen
                preview.reset_last_rect(); // Force MPV window resize
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                if preview.prev_app_maximized() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                }
            }

            // Keyboard shortcuts: volume (Up/Down) and seek (Left/Right)
            // OSD feedback rendered natively by MPV via show-text command
            let vol_step = 0.05_f32;
            let seek_step = 5.0_f64;
            let osd_duration_ms = 2000_i64;

            if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                let new_vol = (volume + vol_step).min(1.0);
                preview.set_volume(new_vol);
                let msg = format!("Volume: {}%", (new_vol * 100.0).round() as i32);
                preview.show_osd(&msg, osd_duration_ms);
                preview.reset_mouse_activity();
            }
            if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                let new_vol = (volume - vol_step).max(0.0);
                preview.set_volume(new_vol);
                let msg = format!("Volume: {}%", (new_vol * 100.0).round() as i32);
                preview.show_osd(&msg, osd_duration_ms);
                preview.reset_mouse_activity();
            }
            if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                let new_time = (current_time + seek_step).min(duration);
                preview.seek(new_time);
                let display_time = crate::ui::components::media_preview::format_time(new_time);
                let display_dur = crate::ui::components::media_preview::format_time(duration);
                let msg = format!("{} / {}", display_time, display_dur);
                preview.show_osd(&msg, osd_duration_ms);
                preview.reset_mouse_activity();
            }
            if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                let new_time = (current_time - seek_step).max(0.0);
                preview.seek(new_time);
                let display_time = crate::ui::components::media_preview::format_time(new_time);
                let display_dur = crate::ui::components::media_preview::format_time(duration);
                let msg = format!("{} / {}", display_time, display_dur);
                preview.show_osd(&msg, osd_duration_ms);
                preview.reset_mouse_activity();
            }

            // Render Video
            let mut video_ui = ui.new_child(egui::UiBuilder::new().max_rect(video_rect));
            preview.set_forced_size(Some(video_rect.size()));
            preview.show(&mut video_ui, frame);

            // Render Controls when active
            if show_controls {
                let control_rect = egui::Rect::from_min_size(
                    egui::pos2(screen_rect.min.x, screen_rect.min.y + video_height),
                    egui::vec2(total_size.x, control_height),
                );

                // Background - use theme-aware colors (same as windowed mode)
                let bg_color = if ui.visuals().dark_mode {
                    egui::Color32::from_rgb(35, 35, 38) // Dark mode panel background
                } else {
                    egui::Color32::from_rgb(245, 245, 248)
                    // Light mode panel background
                };
                ui.painter().rect_filled(control_rect, 0.0, bg_color);

                let mut control_ui = ui.new_child(egui::UiBuilder::new().max_rect(control_rect));
                control_ui.add_space(6.0);
                draw_video_controls(
                    &mut control_ui,
                    preview,
                    control_rect.width() - 20.0,
                    svg_manager,
                    is_playing,
                    current_time,
                    duration,
                    volume,
                    is_muted,
                    true, // is_detached (fullscreen is essentially detached)
                );
            }

            // ESC to exit fullscreen
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                preview.toggle_maximized();
                preview.set_fullscreen_applied(false);
                preview.set_forced_size(None); // Clear forced size when exiting fullscreen
                preview.reset_last_rect(); // Force MPV window resize
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                if preview.prev_app_maximized() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                }
            }

            // Only repaint when video is playing or controls visible (perf optimization)
            if is_playing || preview.controls_active() {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(200));
            }
        });
}
